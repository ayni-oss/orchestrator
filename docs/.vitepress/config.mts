import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(defineConfig({
  title: 'Ayni Orchestrator',
  description: 'Controlled agent workflows from product intent to reviewed software delivery.',
  srcDir: 'site',
  mermaid: {
    startOnLoad: false,
    theme: 'neutral',
    look: 'classic',
    layout: 'elk',
    htmlLabels: false,
    flowchart: { padding: 16, wrappingWidth: 180, useMaxWidth: false },
  },
  base: '/',
  cleanUrls: true,
  lastUpdated: true,
  sitemap: { hostname: 'https://orchestrator.ayni.build' },
  head: [['link', { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }]],
  themeConfig: {
    siteTitle: false,
    nav: [
      { text: 'Overview', link: '/' },
      { text: 'Source', link: 'https://github.com/ayni-oss/orchestrator' },
      { text: 'Ayni CLI', link: 'https://cli.ayni.build' },
    ],
    sidebar: [{ text: 'Ayni Orchestrator', items: [{ text: 'Overview', link: '/' }, { text: 'Connect a repository', link: '/repositories' }, { text: 'Alpha workflow (planned)', link: '/alpha' }] }],
    socialLinks: [{ icon: 'github', link: 'https://github.com/ayni-oss/orchestrator' }],
    editLink: { pattern: 'https://github.com/ayni-oss/orchestrator/edit/alpha/docs/site/:path' },
    search: { provider: 'local' },
  },
}))
