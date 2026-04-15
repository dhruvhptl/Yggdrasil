import asyncio
import base64
import json
import os
import urllib.request
import sys

sys.stdout.reconfigure(encoding='utf-8')
# env vars are loaded by dev.sh before this process starts

from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from scrapling.fetchers import AsyncFetcher, StealthyFetcher, DynamicFetcher
import re
from urllib.parse import urlparse, urljoin, parse_qs
import uvicorn
from youtube_transcript_api import YouTubeTranscriptApi
from googleapiclient.discovery import build as googleapi_build
import fitz  # pymupdf

app = FastAPI()

app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:1420", "tauri://localhost"],
    allow_methods=["*"],
    allow_headers=["*"],
)

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


class PdfFetchRequest(BaseModel):
    pdf_base64: str
    filename: str = ""


class PdfBlock(BaseModel):
    text: str
    page: int


class PdfSection(BaseModel):
    title: str
    heading_level: str  # "chapter" | "section" | "flat"
    page_start: int
    page_end: int
    blocks: list[PdfBlock]


class PdfFetchResponse(BaseModel):
    text: str
    pages: int
    sections: list[PdfSection]


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


MIN_HEADINGS_FOR_STRUCTURE = 2

# Numbered heading patterns: "1", "1.2", "1.2.3", "Chapter 3", "Section 4.1"
# Trailing \S? is optional — a span may contain only the number (e.g. "2.3") with
# the title text in a separate span on the same line.
_NUMBERED_HEADING_RE = re.compile(
    r"^(?:(?:Chapter|Section|Part|Appendix)\s+)?\d+(?:\.\d+)*\.?\s*\S?",
    re.IGNORECASE,
)

def _is_chapter_level(text: str) -> bool:
    """True for top-level headings: 'Chapter N', 'Part N', bare single digit, or 'N.'"""
    return bool(re.match(
        r"^(?:Chapter|Part|Appendix)\s+\d+|^\d+\.\s+\S|^\d+\s+[A-Z]",
        text.strip(), re.IGNORECASE,
    ))


@app.post("/fetch-pdf", response_model=PdfFetchResponse)
async def fetch_pdf(request: PdfFetchRequest):
    try:
        pdf_bytes = base64.b64decode(request.pdf_base64)
    except Exception as e:
        raise HTTPException(status_code=422, detail=f"Invalid base64: {e}")

    try:
        doc = fitz.open(stream=pdf_bytes, filetype="pdf")
    except Exception as e:
        raise HTTPException(status_code=422, detail=f"Could not open PDF: {e}")

    page_count = doc.page_count

    # ── Pass 1: collect all spans with page provenance ──────────────────────
    all_spans: list[dict] = []          # {text, size, flags, bbox, page_no, page_h}
    # line_size_counts: how many lines are dominated by each font size.
    # Using line-dominance rather than character volume avoids math equation
    # micro-spans (subscripts, symbols) skewing the body size estimate.
    line_size_counts: dict[float, int] = {}

    for page_no, page in enumerate(doc, start=1):
        page_h = page.rect.height
        blocks = page.get_text("dict", flags=fitz.TEXT_PRESERVE_WHITESPACE)["blocks"]
        for blk in blocks:
            if blk.get("type") != 0:   # type 0 = text block
                continue
            for line in blk.get("lines", []):
                line_char_by_size: dict[float, int] = {}
                for span in line.get("spans", []):
                    raw = span.get("text", "").replace("\0", "").strip()
                    if not raw:
                        continue
                    size = round(span["size"], 1)
                    line_char_by_size[size] = line_char_by_size.get(size, 0) + len(raw)
                    all_spans.append({
                        "text": raw,
                        "size": size,
                        "flags": span.get("flags", 0),
                        "bbox": span["bbox"],          # (x0, y0, x1, y1)
                        "page_no": page_no,
                        "page_h": page_h,
                    })
                # credit this line to whichever size has the most characters on it
                if line_char_by_size:
                    dominant = max(line_char_by_size, key=lambda s: line_char_by_size[s])
                    line_size_counts[dominant] = line_size_counts.get(dominant, 0) + 1

    # Get TOC before closing — fitz returns [[level, title, page], ...]
    toc = doc.get_toc(simple=True)
    doc.close()

    if not all_spans:
        raise HTTPException(status_code=422, detail="Could not extract meaningful text from PDF")

    # ── Body font size = size dominating the most lines ──────────────────────
    # Line-dominance is robust against equation micro-spans inflating small sizes.
    body_size = max(line_size_counts, key=lambda s: line_size_counts[s]) if line_size_counts else 10.0

    # ── Filter repeated header/footer text (appears on 3+ pages at same y%) ─
    # Key: (rounded_y_pct, text) → set of page numbers
    _pos_text_pages: dict[tuple, set] = {}
    for sp in all_spans:
        y_pct = sp["bbox"][1] / sp["page_h"] if sp["page_h"] else 0.5
        if y_pct < 0.08 or y_pct > 0.92:
            key = (round(y_pct, 2), sp["text"])
            _pos_text_pages.setdefault(key, set()).add(sp["page_no"])
    repeated = {text for (_, text), pages in _pos_text_pages.items() if len(pages) >= 3}

    # ── TOC fast-path: use embedded TOC if it has enough entries ─────────────
    # doc.get_toc() returns [[level, title, page], ...] (1-indexed pages).
    # This is authoritative for well-structured PDFs like textbooks and reports.
    if len(toc) >= MIN_HEADINGS_FOR_STRUCTURE:
        print(f"[scraper/pdf-toc] Using embedded TOC: {len(toc)} entries")

        # Build sections from TOC entries, assign page ranges
        toc_sections: list[dict] = []
        for i, (level, title, page_start) in enumerate(toc):
            page_end = toc[i + 1][2] - 1 if i + 1 < len(toc) else page_count
            page_end = max(page_start, page_end)  # guard against same-page entries
            toc_sections.append({
                "title": title.strip(),
                "heading_level": "chapter" if level == 1 else "section",
                "page_start": page_start,
                "page_end": page_end,
                "blocks": [],
            })

        # Assign body spans to sections by page range
        for sp in all_spans:
            if sp["text"] in repeated:
                continue
            p = sp["page_no"]
            for sec in toc_sections:
                if sec["page_start"] <= p <= sec["page_end"]:
                    sec["blocks"].append({"text": sp["text"], "page": p})
                    break

        # Build output, skipping empty sections
        sections_out = [
            PdfSection(
                title=sec["title"],
                heading_level=sec["heading_level"],
                page_start=sec["page_start"],
                page_end=sec["page_end"],
                blocks=[PdfBlock(text=b["text"], page=b["page"]) for b in sec["blocks"]],
            )
            for sec in toc_sections if sec["blocks"]
        ]

        flat_text = "\n\n".join(
            sec["title"] + "\n" + " ".join(b["text"] for b in sec["blocks"])
            for sec in toc_sections if sec["blocks"]
        ).strip()

        if len(flat_text) < 50:
            raise HTTPException(status_code=422, detail="Could not extract meaningful text from PDF")

        print(
            f"[scraper] PDF '{request.filename}': {page_count} pages, "
            f"{len(toc)} TOC entries → {len(sections_out)} sections, {len(flat_text)} chars"
        )
        return PdfFetchResponse(text=flat_text, pages=page_count, sections=sections_out)

    # ── Pass 2: classify each span as heading or body block (font heuristics) ─
    # A span is bold if bit 4 (16) of flags is set
    def is_bold(flags: int) -> bool:
        return bool(flags & 16)

    def classify_span(sp: dict) -> str:
        """Returns 'chapter', 'section', or 'body'."""
        if sp["text"] in repeated:
            return "body"
        size_diff = sp["size"] - body_size
        bold = is_bold(sp["flags"])
        text = sp["text"].strip()
        numbered = bool(_NUMBERED_HEADING_RE.match(text))
        # Reject pure numeric strings — page numbers, footnote markers, equation labels
        if numbered and not re.search(r'[A-Za-z]', text):
            numbered = False

        # Must be visually heading-sized or bold (relaxed from <=2 to <=1)
        if size_diff <= 1 and not bold:
            return "body"
        # Unnumbered headings require a larger size diff (relaxed from <=4 to <=3)
        if not numbered and size_diff <= 3:
            return "body"

        if _is_chapter_level(text):
            return "chapter"
        return "section"

    # ── Merge consecutive body spans into paragraph blocks ───────────────────
    # We group by (page_no) continuity; a heading resets the group.
    headings: list[dict] = []   # {text, level, page_no}
    body_blocks: list[dict] = []  # accumulated blocks between headings

    # We'll build sections as we scan
    raw_sections: list[dict] = []   # {title, heading_level, blocks:[{text,page}]}
    current_title = "[Start]"
    current_level = "flat"
    current_blocks: list[dict] = []

    for sp in all_spans:
        cls = classify_span(sp)
        if cls in ("chapter", "section"):
            if current_blocks or current_title != "[Start]":
                raw_sections.append({
                    "title": current_title,
                    "heading_level": current_level,
                    "blocks": list(current_blocks),
                })
            current_title = sp["text"]
            current_level = cls
            current_blocks = []
        else:
            if sp["text"] not in repeated:
                current_blocks.append({"text": sp["text"], "page": sp["page_no"]})

    # Flush last section
    if current_blocks:
        raw_sections.append({
            "title": current_title,
            "heading_level": current_level,
            "blocks": list(current_blocks),
        })

    # ── Debug: report top 20 heading candidates and why they passed/failed ──────
    # Collect all spans that were above body size or bold (potential headings)
    candidates_debug: list[dict] = []
    for sp in all_spans:
        if sp["text"] in repeated:
            continue
        size_diff = sp["size"] - body_size
        bold = is_bold(sp["flags"])
        if size_diff <= 0 and not bold:
            continue  # no signal at all — skip from debug output
        text = sp["text"].strip()
        numbered = bool(_NUMBERED_HEADING_RE.match(text))
        fail_reason = None
        if size_diff <= 2 and not bold:
            fail_reason = f"size_diff={size_diff:.1f}<=2 and not bold"
        elif not numbered and size_diff <= 4:
            fail_reason = f"unnumbered and size_diff={size_diff:.1f}<=4"
        candidates_debug.append({
            "text": text[:80],
            "size": sp["size"],
            "size_diff": size_diff,
            "bold": bold,
            "numbered": numbered,
            "page": sp["page_no"],
            "result": fail_reason or ("chapter" if _is_chapter_level(text) else "section"),
        })

    # Sort by size_diff desc, take top 20
    candidates_debug.sort(key=lambda x: x["size_diff"], reverse=True)
    print(f"[scraper/pdf-debug] body_size={body_size}pt, repeated_strings={len(repeated)}, "
          f"heading_candidates={len(candidates_debug)}")
    for c in candidates_debug[:20]:
        status = "✅ PASS" if c["result"] in ("chapter", "section") else f"❌ FAIL: {c['result']}"
        print(
            f"  {status} | p{c['page']} | size={c['size']}pt (diff={c['size_diff']:+.1f}) | "
            f"bold={c['bold']} numbered={c['numbered']} | \"{c['text']}\""
        )
    detected = len(real_headings := [s for s in raw_sections if s["heading_level"] in ("chapter", "section")])
    print(f"[scraper/pdf-debug] {detected} headings passed → "
          f"{'structured' if detected >= MIN_HEADINGS_FOR_STRUCTURE else 'FLAT FALLBACK'}")

    # ── Determine whether structure is meaningful ─────────────────────────────

    if len(real_headings) < MIN_HEADINGS_FOR_STRUCTURE:
        # Fall back: single flat section with all text
        flat_text = " ".join(
            sp["text"] for sp in all_spans if sp["text"] not in repeated
        ).strip()
        if len(flat_text) < 50:
            raise HTTPException(status_code=422, detail="Could not extract meaningful text from PDF")
        sections_out = [PdfSection(
            title="Full Text",
            heading_level="flat",
            page_start=1,
            page_end=page_count,
            blocks=[PdfBlock(text=flat_text, page=1)],
        )]
        print(f"[scraper] PDF '{request.filename}': {page_count} pages, flat fallback ({len(flat_text)} chars)")
        return PdfFetchResponse(text=flat_text, pages=page_count, sections=sections_out)

    # ── Build final sections with page provenance ─────────────────────────────
    sections_out: list[PdfSection] = []
    for sec in raw_sections:
        if not sec["blocks"]:
            continue
        pages_in_sec = [b["page"] for b in sec["blocks"]]
        sections_out.append(PdfSection(
            title=sec["title"],
            heading_level=sec["heading_level"],
            page_start=min(pages_in_sec),
            page_end=max(pages_in_sec),
            blocks=[PdfBlock(text=b["text"], page=b["page"]) for b in sec["blocks"]],
        ))

    flat_text = "\n\n".join(
        sec["title"] + "\n" + " ".join(b["text"] for b in sec["blocks"])
        for sec in raw_sections if sec["blocks"]
    ).strip()

    heading_count = len([s for s in sections_out if s.heading_level in ("chapter", "section")])
    print(
        f"[scraper] PDF '{request.filename}': {page_count} pages, "
        f"{heading_count} headings, {len(sections_out)} sections, {len(flat_text)} chars"
    )
    return PdfFetchResponse(text=flat_text, pages=page_count, sections=sections_out)


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
