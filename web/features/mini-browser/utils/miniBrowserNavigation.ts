/**
 * Route constants for the embedded mini browser workbench.
 *
 * The page itself lives in `pages/MiniBrowserPage.tsx`; the toolbar entry in
 * `components/MiniBrowserButton.tsx` and the layout guard in
 * `components/layout/MainLayout/index.tsx` both consume these helpers so the
 * path literal exists in exactly one place.
 */
export const MINI_BROWSER_PAGE_PATH = '/mini-browser';

export const isMiniBrowserPath = (pathname: string): boolean =>
  pathname === MINI_BROWSER_PAGE_PATH
  || pathname.startsWith(`${MINI_BROWSER_PAGE_PATH}/`);
