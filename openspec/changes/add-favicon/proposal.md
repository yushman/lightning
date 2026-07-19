# add-favicon

## Why

Every UI page load logs a `404` for `/favicon.ico` (observed during landing-page screenshotting), and browser tabs show a blank icon.

## What Changes

- The server serves an embedded SVG favicon (lightning bolt) at `/favicon.svg` and at the browsers' default `/favicon.ico` probe; all HTML pages reference it via `<link rel="icon">`.

## Capabilities

### Modified Capabilities

- `web-ui`: favicon served and referenced.

## Impact

crates/server (web.rs, main.rs) only.
