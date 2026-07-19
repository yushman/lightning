## ADDED Requirements

### Requirement: Favicon
The server SHALL serve an embedded SVG favicon at `/favicon.svg` and at `/favicon.ico` (SVG body, `image/svg+xml` content type), and every HTML page SHALL reference it with a `<link rel="icon">` tag, so no page load produces a favicon 404.

#### Scenario: Favicon served
- **WHEN** a browser requests `/favicon.svg` or `/favicon.ico`
- **THEN** the server responds 200 with `image/svg+xml` content
