# Navop documentation

The public Navop usage guide is built with VitePress and deployed to GitHub Pages.

```bash
npm install
npm run dev
npm run build
```

The production custom domain is `https://docs.navop.dev`. Simplified Chinese is served from the root path, with Traditional Chinese under `/zh-TW/` and English under `/en-US/`.

The GitHub Pages workflow deploys changes from the repository's `dev` branch. Do not place internal engineering plans in this directory; `docs-site/docs` is public end-user documentation.
