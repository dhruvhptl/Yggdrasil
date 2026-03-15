import asyncio
import json
import os
import pathlib
import urllib.request
import sys

sys.stdout.reconfigure(encoding='utf-8')
from dotenv import load_dotenv

# Load env vars from src-tauri/.env so YOUTUBE_API_KEY etc. are available
load_dotenv(pathlib.Path(__file__).parent.parent / "src-tauri" / ".env")

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from scrapling.fetchers import AsyncFetcher, StealthyFetcher, DynamicFetcher
import re
from urllib.parse import urlparse, urljoin, parse_qs
import uvicorn
from youtube_transcript_api import YouTubeTranscriptApi
from googleapiclient.discovery import build as googleapi_build

app = FastAPI()

YOUTUBE_API_KEY = os.getenv("YOUTUBE_API_KEY", "")

# Limit concurrent browser instances (Tier 2 + Tier 3)
BROWSER_SEMAPHORE = asyncio.Semaphore(3)


class FetchRequest(BaseModel):
    url: str
    force_dynamic: bool = False


class ExternalLink(BaseModel):
    url: str
    text: str
    type: str  # "youtube" | "url"


class FetchResponse(BaseModel):
    text: str
    fetcher_used: str
    char_count: int
    page_type: str = "article"
    external_links: list[ExternalLink] = []


class DiscoverRequest(BaseModel):
    url: str


class DiscoveredLink(BaseModel):
    url: str
    text: str


class DiscoverResponse(BaseModel):
    source_url: str
    links: list[DiscoveredLink]


class PlaylistVideo(BaseModel):
    video_id: str
    title: str
    url: str
    position: int


class PlaylistResponse(BaseModel):
    playlist_title: str
    playlist_id: str
    videos: list[PlaylistVideo]


def extract_text(page) -> str:
    """Extract clean text from a Scrapling page object."""
    text_parts = []
    seen_texts = set()

    def add(text: str):
        t = text.strip()
        if t and t not in seen_texts:
            seen_texts.add(t)
            text_parts.append(t)

    # Headings
    for tag in ['h1', 'h2', 'h3', 'h4']:
        for el in page.css(tag):
            t = el.text.strip()
            if t:
                add(f"\n## {t}\n")

    # Paragraphs
    for el in page.css('p'):
        t = el.text.strip()
        if t and len(t) > 30:
            add(t)

    # Code blocks
    for el in page.css('pre, code'):
        t = el.text.strip()
        if t:
            add(f"\n```\n{t}\n```\n")

    # List items with link preservation
    for el in page.css('li'):
        t = el.text.strip()
        if not t:
            continue
        links = []
        for a in el.css('a'):
            href = a.attrib.get('href', '')
            if href and href.startswith('http'):
                links.append(href)
        if links:
            add(f"• {t} [{', '.join(links)}]")
        else:
            add(f"• {t}")

    # Table cells — for old table-based layouts
    for el in page.css('td'):
        t = el.text.strip()
        if t and len(t) > 20 and t not in seen_texts:
            links = []
            for a in el.css('a'):
                href = a.attrib.get('href', '')
                if href and href.startswith('http'):
                    links.append(href)
            if links:
                add(f"• {t} [{', '.join(links)}]")
            else:
                add(t)

    # Standalone anchor links — only external URLs not already captured
    # Skip nav, footer, header anchors
    for el in page.css('main a, article a, section a, .content a, .post a, .entry a'):
        href = el.attrib.get('href', '')
        t = el.text.strip()
        if (t and href and href.startswith('http')
                and t not in seen_texts
                and len(t) > 5
                and len(t) < 200):
            seen_texts.add(t)
            add(f"-> {t} [{href}]")

    # Second pass for class-substring selectors — wrapped individually to survive parse errors
    for selector in ['[class*="module"] a', '[class*="section"] a', '[class*="card"] a']:
        try:
            for el in page.css(selector):
                href = el.attrib.get('href', '')
                t = el.text.strip()
                if (t and href and href.startswith('http')
                        and t not in seen_texts
                        and len(t) > 5
                        and len(t) < 200):
                    seen_texts.add(t)
                    add(f"-> {t} [{href}]")
        except Exception:
            pass

    text = '\n'.join(text_parts)
    text = re.sub(r'\n{3,}', '\n\n', text).strip()
    return text


def is_insufficient(text: str) -> bool:
    return len(text.strip()) < 300


_NAV_SEGMENTS = {
    'login', 'signup', 'sign-up', 'register', 'search', 'pricing',
    'about', 'contact', 'terms', 'privacy', 'legal', 'careers',
}


def extract_links(page, source_url: str) -> list[dict]:
    """Extract sub-page links that are part of the source content, not site nav."""
    parsed_source = urlparse(source_url)
    source_domain = parsed_source.netloc
    source_norm = source_url.split('#')[0].rstrip('/')

    # Build path prefix filter when the source has 2+ path segments
    source_path_segs = [s for s in parsed_source.path.split('/') if s]
    use_prefix_filter = len(source_path_segs) >= 2
    source_path_prefix = '/' + '/'.join(source_path_segs)   # e.g. /user/repo
    source_path_depth = len(source_path_segs)

    # "blog" in source path means /blog links are content, not nav
    source_is_blog = 'blog' in source_path_segs

    seen_urls: set[str] = set()
    links = []

    for el in page.css('a'):
        try:
            href = el.attrib.get('href', '').strip()
        except Exception:
            continue

        if not href or href.startswith('#') or href.startswith('mailto:') or href.startswith('javascript:'):
            continue

        abs_url = urljoin(source_url, href)
        parsed_abs = urlparse(abs_url)

        if parsed_abs.netloc != source_domain:
            continue
        if parsed_abs.scheme not in ('http', 'https'):
            continue

        clean_url = abs_url.split('#')[0].rstrip('/')
        if not clean_url or clean_url == source_norm:
            continue

        if clean_url in seen_urls:
            continue

        link_path_segs = [s for s in parsed_abs.path.split('/') if s]
        link_path = '/' + '/'.join(link_path_segs) if link_path_segs else '/'

        # Prefix filter: for deep source pages, only keep links under the same path
        if use_prefix_filter and not link_path.startswith(source_path_prefix + '/'):
            continue

        # Depth filter: never crawl up to a parent page
        if len(link_path_segs) < source_path_depth:
            continue

        # Nav/utility path filter — check every segment of the link path
        if any(seg in _NAV_SEGMENTS for seg in link_path_segs):
            # Allow /blog/* only when source itself is under /blog
            if not (source_is_blog and link_path_segs and link_path_segs[0] == 'blog'):
                continue

        seen_urls.add(clean_url)

        try:
            text = (el.text or '').strip()
        except Exception:
            text = ''
        if not text:
            path = parsed_abs.path.strip('/')
            text = path.split('/')[-1] if path else clean_url

        links.append({'url': clean_url, 'text': text[:100]})
        if len(links) >= 50:
            break

    return links


# ─── Page type detection & external link extraction ──────────────────────────

_NOISE_PATTERNS = (
    'doubleclick', 'facebook.com/share', 'twitter.com/intent',
    'linkedin.com/share', 'addthis', 'buymeacoffee', 'forms.gle',
)

_IMAGE_EXTENSIONS = ('.png', '.jpg', '.jpeg', '.gif', '.svg', '.webp')


def detect_page_type(url: str, page, text: str) -> str:
    """Classify a page into one of the known types."""
    parsed = urlparse(url)
    host = parsed.netloc.lower().replace('www.', '')
    path_segs = [s for s in parsed.path.split('/') if s]

    # 1. GitHub repo (exactly 2 path segments like /user/repo)
    if 'github.com' in host and len(path_segs) == 2:
        return "github_repo"

    # 2. YouTube video
    if _is_youtube_video(url):
        return "youtube_video"

    # 3. YouTube playlist
    if _is_youtube_playlist(url):
        return "youtube_playlist"

    # 4. Documentation
    doc_segments = {'docs', 'wiki', 'api'}
    doc_domains = ('readthedocs.io', 'gitbook.io')
    if any(seg in doc_segments for seg in path_segs):
        return "documentation"
    if any(host.endswith(d) for d in doc_domains):
        return "documentation"

    # 5. Resource list — high ratio of external links to words
    source_domain = parsed.netloc.lower()
    external_count = 0
    try:
        for el in page.css('a'):
            href = el.attrib.get('href', '').strip()
            if not href or not href.startswith('http'):
                continue
            link_domain = urlparse(href).netloc.lower()
            if link_domain and link_domain != source_domain:
                external_count += 1
    except Exception:
        pass

    word_count = len(text.split())
    if external_count >= 5 and external_count / max(word_count, 1) > 0.01:
        return "resource_list"

    # 6. Default
    return "article"


def extract_external_links(page, source_url: str) -> list[dict]:
    """Extract deduplicated external links from a page."""
    source_domain = urlparse(source_url).netloc.lower()
    seen_urls: set[str] = set()
    links: list[dict] = []

    for el in page.css('a'):
        try:
            href = el.attrib.get('href', '').strip()
        except Exception:
            continue

        if not href or href.startswith('#') or href.startswith('mailto:') or href.startswith('javascript:'):
            continue

        abs_url = urljoin(source_url, href)
        parsed = urlparse(abs_url)

        if parsed.scheme not in ('http', 'https'):
            continue

        link_domain = parsed.netloc.lower()
        if not link_domain or link_domain == source_domain:
            continue

        # Skip images
        path_lower = parsed.path.lower()
        if any(path_lower.endswith(ext) for ext in _IMAGE_EXTENSIONS):
            continue

        # Skip noise (ads, social share, tracking)
        if any(noise in abs_url.lower() for noise in _NOISE_PATTERNS):
            continue

        clean_url = abs_url.split('#')[0].rstrip('/')
        if clean_url in seen_urls:
            continue
        seen_urls.add(clean_url)

        # Anchor text
        try:
            text = (el.text or '').strip()
        except Exception:
            text = ''
        if not text:
            text = parsed.path.strip('/').split('/')[-1] if parsed.path.strip('/') else clean_url
        text = text[:200].strip()

        # Classify type
        link_type = "youtube" if ('youtube.com/watch' in clean_url or 'youtu.be/' in clean_url) else "url"

        links.append({"url": clean_url, "text": text, "type": link_type})
        if len(links) >= 100:
            break

    return links


# ─── YouTube helpers ──────────────────────────────────────────────────────────

def _is_youtube_video(url: str) -> bool:
    parsed = urlparse(url)
    host = parsed.netloc.lower().replace('www.', '')
    if host == 'youtu.be':
        return True
    if host in ('youtube.com', 'm.youtube.com'):
        params = parse_qs(parsed.query)
        return 'v' in params
    return False


def _is_youtube_playlist(url: str) -> bool:
    parsed = urlparse(url)
    host = parsed.netloc.lower().replace('www.', '')
    if host in ('youtube.com', 'm.youtube.com'):
        params = parse_qs(parsed.query)
        return 'list' in params and parsed.path.startswith('/playlist')
    return False


def _extract_video_id(url: str) -> str | None:
    parsed = urlparse(url)
    host = parsed.netloc.lower().replace('www.', '')
    if host == 'youtu.be':
        return parsed.path.lstrip('/').split('/')[0] or None
    if host in ('youtube.com', 'm.youtube.com'):
        params = parse_qs(parsed.query)
        v = params.get('v', [])
        return v[0] if v else None
    return None


def _extract_playlist_id(url: str) -> str | None:
    params = parse_qs(urlparse(url).query)
    lst = params.get('list', [])
    return lst[0] if lst else None


def _format_transcript(segments) -> str:
    """Join transcript segments into paragraphs, breaking every ~800 chars at a word boundary."""
    texts = []
    for seg in segments:
        if isinstance(seg, dict):
            t = seg.get('text', '').strip()
        else:
            t = getattr(seg, 'text', '').strip()
        if t:
            texts.append(t)
    full = ' '.join(texts)

    result = []
    pos = 0
    while pos < len(full):
        end = pos + 800
        if end >= len(full):
            result.append(full[pos:])
            break
        space = full.rfind(' ', pos, end)
        if space <= pos:
            space = end
        result.append(full[pos:space])
        pos = space + 1
    return '\n\n'.join(result)


def _fetch_youtube_transcript(video_id: str, url: str) -> FetchResponse:
    """Fetch transcript + oEmbed metadata for a YouTube video. Runs in a thread."""
    # Fetch title / channel via oEmbed
    title = f"YouTube: {video_id}"
    author = ""
    try:
        oembed_url = f"https://www.youtube.com/oembed?url={url}&format=json"
        req = urllib.request.Request(oembed_url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(req, timeout=5) as resp:
            data = json.loads(resp.read())
            title = data.get("title", title)
            author = data.get("author_name", "")
    except Exception:
        pass

    segments = None

    # Attempt 1: English transcript (class-method API — works on v0.x and v1.x)
    try:
        segments = list(YouTubeTranscriptApi.get_transcript(video_id, languages=["en", "en-US", "en-GB"]))
    except Exception:
        pass

    # Attempt 2: any available language (instance API)
    if segments is None:
        try:
            ytt_api = YouTubeTranscriptApi()
            transcript_list = ytt_api.list(video_id)
            for t in transcript_list:
                try:
                    fetched = t.fetch()
                    segments = list(fetched)
                    break
                except Exception:
                    continue
        except Exception:
            pass

    header = f"# {title}"
    if author:
        header += f"\nChannel: {author}"
    header += "\n\n"

    if segments:
        full_text = header + _format_transcript(segments)
        print(f"[scraper] Fetching {url} with YouTubeTranscript")
        return FetchResponse(text=full_text, fetcher_used="YouTubeTranscript", char_count=len(full_text))
    else:
        full_text = "[No transcript available — metadata only]\n" + header
        print(f"[scraper] Fetching {url} with YouTubeTranscript (metadata only)")
        return FetchResponse(text=full_text, fetcher_used="YouTubeTranscript", char_count=len(full_text))


def _fetch_playlist_sync(playlist_id: str) -> PlaylistResponse:
    """Fetch playlist metadata + video list from YouTube Data API. Runs in a thread."""
    youtube = googleapi_build("youtube", "v3", developerKey=YOUTUBE_API_KEY)

    # Fetch playlist title
    pl_resp = youtube.playlists().list(part="snippet", id=playlist_id).execute()
    playlist_title = (
        pl_resp["items"][0]["snippet"]["title"]
        if pl_resp.get("items")
        else playlist_id
    )

    # Fetch all videos with pagination, capped at 200
    videos: list[PlaylistVideo] = []
    next_page_token = None

    while len(videos) < 200:
        req = youtube.playlistItems().list(
            part="snippet",
            playlistId=playlist_id,
            maxResults=50,
            pageToken=next_page_token,
        )
        resp = req.execute()

        for item in resp.get("items", []):
            snippet = item["snippet"]
            resource = snippet.get("resourceId", {})
            vid_id = resource.get("videoId", "")
            if not vid_id:
                continue
            videos.append(PlaylistVideo(
                video_id=vid_id,
                title=snippet.get("title", ""),
                url=f"https://www.youtube.com/watch?v={vid_id}",
                position=snippet.get("position", len(videos)),
            ))

        next_page_token = resp.get("nextPageToken")
        if not next_page_token:
            break

    return PlaylistResponse(
        playlist_title=playlist_title,
        playlist_id=playlist_id,
        videos=videos[:200],
    )


def _build_response(url: str, text: str, fetcher: str, page) -> FetchResponse:
    """Build FetchResponse with page type detection and optional external links."""
    page_type = detect_page_type(url, page, text)
    ext_links: list[ExternalLink] = []
    if page_type == "resource_list":
        ext_links = [ExternalLink(**l) for l in extract_external_links(page, url)]
    print(f"[scraper] {fetcher} → {page_type}, {len(ext_links)} external links")
    return FetchResponse(
        text=text,
        fetcher_used=fetcher,
        char_count=len(text),
        page_type=page_type,
        external_links=ext_links,
    )


@app.post("/fetch", response_model=FetchResponse)
async def fetch_url(request: FetchRequest):
    url = request.url

    # YouTube video: use transcript API instead of scraping
    if _is_youtube_video(url):
        video_id = _extract_video_id(url)
        if video_id:
            resp = await asyncio.to_thread(_fetch_youtube_transcript, video_id, url)
            resp.page_type = "youtube_video"
            return resp

    # Tier 1: Fast async HTTP with browser TLS fingerprinting (no semaphore — no browser)
    if not request.force_dynamic:
        try:
            page = await AsyncFetcher.get(url, stealthy_headers=True, timeout=15000)
            text = extract_text(page)
            print(f"[scraper] AsyncFetcher got {len(text)} chars for {url}")
            if not is_insufficient(text):
                print(f"[scraper] Fetching {url} with AsyncFetcher")
                return _build_response(url, text, "AsyncFetcher", page)
            else:
                print(f"[scraper] AsyncFetcher insufficient, escalating")
        except Exception:
            pass

    # Tier 2: Stealthy browser fetcher — bypasses Cloudflare, bot detection
    if not request.force_dynamic:
        try:
            async with BROWSER_SEMAPHORE:
                page = await StealthyFetcher.async_fetch(url, headless=True, network_idle=True, timeout=30000)
            text = extract_text(page)
            if not is_insufficient(text):
                print(f"[scraper] Fetching {url} with StealthyFetcher")
                return _build_response(url, text, "StealthyFetcher", page)
        except Exception:
            pass

    # Tier 3: Full browser automation — JS-heavy sites
    try:
        async with BROWSER_SEMAPHORE:
            page = await DynamicFetcher.async_fetch(url, headless=True, network_idle=True, timeout=45000)
        text = extract_text(page)
        if is_insufficient(text):
            raise HTTPException(
                status_code=422,
                detail="Could not extract meaningful text from this URL. Try pasting the content as text instead.",
            )
        print(f"[scraper] Fetching {url} with DynamicFetcher")
        return _build_response(url, text, "DynamicFetcher", page)
    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(
            status_code=500,
            detail=f"All fetchers failed: {e}. Try pasting the content as text instead.",
        )


@app.post("/fetch-playlist", response_model=PlaylistResponse)
async def fetch_playlist(request: DiscoverRequest):
    url = request.url
    playlist_id = _extract_playlist_id(url)
    if not playlist_id:
        raise HTTPException(status_code=422, detail="Could not extract playlist ID from URL")
    if not YOUTUBE_API_KEY:
        raise HTTPException(
            status_code=422,
            detail="YouTube API key not configured. Add YOUTUBE_API_KEY to .env",
        )
    return await asyncio.to_thread(_fetch_playlist_sync, playlist_id)


@app.post("/discover", response_model=DiscoverResponse)
async def discover_url(request: DiscoverRequest):
    url = request.url
    page = None

    # Tier 1: Fast async HTTP
    try:
        page = await AsyncFetcher.get(url, stealthy_headers=True, timeout=15000)
    except Exception:
        pass

    # Tier 2: Stealthy browser
    if page is None:
        try:
            async with BROWSER_SEMAPHORE:
                page = await StealthyFetcher.async_fetch(url, headless=True, network_idle=True, timeout=30000)
        except Exception:
            pass

    # Tier 3: Full browser
    if page is None:
        try:
            async with BROWSER_SEMAPHORE:
                page = await DynamicFetcher.async_fetch(url, headless=True, network_idle=True, timeout=45000)
        except Exception as e:
            raise HTTPException(status_code=502, detail=f"Could not fetch page: {e}")

    links = extract_links(page, url)
    print(f"[scraper] Discovered {len(links)} links from {url}")
    return DiscoverResponse(source_url=url, links=[DiscoveredLink(**l) for l in links])


@app.get("/health")
async def health():
    return {"status": "ok", "name": "Yggdrasil Scraper", "port": 3002}

@app.post("/debug")
async def debug_url(request: FetchRequest):
    async with BROWSER_SEMAPHORE:
        page = await DynamicFetcher.async_fetch(request.url, headless=True, network_idle=True, timeout=45000)
    html = page.html_content if hasattr(page, 'html_content') else str(page)
    return {"html": str(html)[:5000]}

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=3002)
