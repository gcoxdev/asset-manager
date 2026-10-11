# Asset Manager website

A static product site and complete user guide. Node 22+ generates plain HTML,
CSS, JavaScript and images into `dist/`. There are no npm dependencies,
runtime API calls to GitHub, external fonts, analytics or application backend.

## Work locally

From the repository root:

```sh
npm --prefix website run build
npm --prefix website test
npm --prefix website run preview
```

Open `http://127.0.0.1:4321`. Rebuild after editing source files. Stop the
preview with Ctrl+C. The site is designed to be served at a domain's root.

## Keep the guide and screenshots current

The source of truth is `apps/desktop/src/help/topics.js`. The website contains
a committed snapshot so its folder can be built independently of the app.
To refresh the snapshot, search implementation, app icon and screenshots:

```sh
npm --prefix website run sync:app
npm --prefix website run check:sync
npm --prefix website test
```

Commit the synced files with the app help changes. The website CI checks for
drift. Article bodies, all 73 current topics, and cross-topic links mirror the
app. In-app screen links become labels because this site cannot open the app.
Search uses the app's own ranking and synonyms, runs locally in the browser,
and supports linkable queries. All topics remain readable without JavaScript.

The three screenshots are copied from `docs/screenshots/`. Their source is
the fictional catalog in `apps/desktop/src-tauri/tests/demo_fixtures.rs`,
captured by `apps/desktop/tests-ui/screenshots.spec.mjs`. No real vault data
is read. To regenerate them, use the app's documented `screenshots` command,
then sync the website. Review images before committing them. Never substitute
screenshots of a personal vault. The public captions identify the dummy data.

## Update downloads

`content/release.json` selects a **published** release, its preview status,
and exact GitHub asset filenames. It is deliberately separate from the app's
development version. Update it after publishing a release, verify every
asset exists, and rebuild. The current release is the published `v0.0.3`
preview. A `/releases/latest` redirect excludes pre-releases, so the site
uses explicit versioned links and offers an “All releases” link.

## Structure

- `scripts/pages.mjs`: product copy, page templates and navigation.
- `scripts/markup.mjs`: escaped rendering of the app's help format.
- `public/assets/`: styles, browser enhancements, icon and demo screenshots.
- `content/help.json`: synchronized help content; edit the app source instead.
- `content/release.json`: published download version and asset names.
- `tests/`: renderer, routing, asset and content consistency checks.
- `Dockerfile`, `nginx.conf`: build only this folder and serve only `dist/`.

Generated output is ignored. The build does not need Rust, the parent folder,
an npm install, credentials, or network access. The container build needs to
download its Node and Nginx base images. Production serves real directories
for guide links and returns an actual 404 for unknown paths.

Website code and copied project assets use the repository's AGPL-3.0 license.
