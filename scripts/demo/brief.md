# ENG-270: paginate the search results page

## Decisions to approve

- **D1.** Page with an opaque cursor, not an offset, so new products never shift a page.
- **D2.** Show 24 results a page, the grid's three full rows on a desktop.
- **D3.** Keep the `?page=` links working: redirect them to the first page.

## Acceptance criteria

1. A search with no cursor shows the first 24 results and a Next link.
2. The last page shows no Next link.
3. An old `?page=3` link lands on the first page with a 301.

## Plan

1. Add `cursor` and `limit` to `search/query.rs`, keyed on the result's rank and id.
2. Render Next and Previous links in `templates/search.html`.
3. Cover the three criteria in `tests/search_pages.rs`.

Risk: the rank changes when the index rebuilds, so a cursor can skip a result during a rebuild.
