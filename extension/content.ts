// Content script: extract job context from the current page.
// Responds to { type: "GET_PAGE_CONTEXT" } messages from the popup.

interface PageContext {
  company: string;
  position: string;
}

function getMetaContent(selector: string): string {
  const el = document.querySelector(selector) as HTMLMetaElement | null;
  return el?.content ?? '';
}

function parseJobTitle(title: string): PageContext {
  // "Software Engineer at Google" or "SWE Intern at Stripe, Inc."
  const atMatch = title.match(/^(.+?)\s+at\s+(.+?)(?:\s*[\|\-\–].*)?$/i);
  if (atMatch) {
    return { position: atMatch[1].trim(), company: atMatch[2].trim() };
  }

  // "Google - Software Engineer" or "Google | SWE Intern"
  const dashMatch = title.match(/^(.+?)\s*[\-\–\|]\s*(.+?)(?:\s*[\|\-\–].*)?$/);
  if (dashMatch) {
    return { company: dashMatch[1].trim(), position: dashMatch[2].trim() };
  }

  return { company: '', position: title.trim() };
}

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg.type !== 'GET_PAGE_CONTEXT') return;

  const ogTitle = getMetaContent('meta[property="og:title"]');
  const twitterTitle = getMetaContent('meta[name="twitter:title"]');
  const title = ogTitle || twitterTitle || document.title;

  sendResponse(parseJobTitle(title));
});
