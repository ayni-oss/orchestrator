import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(defineConfig({
  title: 'Ayni Orchestrator',
  description: 'Turn a development request into a reviewable pull request.',
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
      { text: 'Get started', link: '/getting-started' },
      { text: 'Run a task', link: '/projects-and-tasks' },
      { text: 'Troubleshooting', link: '/troubleshooting' },
      { text: 'Ayni CLI', link: 'https://cli.ayni.build' },
    ],
    sidebar: [
      { text: 'Ayni Orchestrator', items: [
        { text: 'Home', link: '/' },
        { text: 'How it works', link: '/how-it-works' },
        { text: 'Get started', link: '/getting-started' },
        { text: 'Repositories and runtimes', link: '/repositories-and-runtimes' },
        { text: 'Projects and tasks', link: '/projects-and-tasks' },
        { text: 'Results and recovery', link: '/results-and-recovery' },
        { text: 'Trust and quality', link: '/trust-and-quality' },
        { text: 'Troubleshooting', link: '/troubleshooting' },
      ] },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/ayni-oss/orchestrator' }],
    editLink: { pattern: 'https://github.com/ayni-oss/orchestrator/edit/main/docs/site/:path' },
    search: { provider: 'local' },
  },
}))
