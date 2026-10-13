import path from 'node:path'
import fs from 'node:fs/promises'
import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

const sections = [
  ['Guides', 'guides', ['ayni-product-flow', 'operator-onboarding', 'managed-runtime-execution', 'environment-changes']],
  ['Architecture', 'architecture', ['architecture', 'code-organization', 'event-platform', 'assurance', 'runtime-runner', 'design-system']],
  ['Operations', 'operations', ['local-development', 'operations', 'event-compatibility', 'github-security']],
  ['Reference', 'reference', ['ayni-integration', 'repository-builders', 'custom-client-access', 'verification-events', 'runtime-operation-events', 'coding-events', 'operator-projections', 'observability-and-events', 'cloudevents-contract']],
] as const

export default withMermaid(defineConfig({
  title: 'Ayni Orchestrator',
  description: 'Worker-owned development tasks on verified Ayni runtimes.',
  base: '/',
  cleanUrls: true,
  lastUpdated: true,
  sitemap: { hostname: 'https://orchestrator.ayni.build' },
  head: [['link', { rel: 'icon', href: '/ayni-logo.svg' }]],
  themeConfig: {
    siteTitle: false,
    nav: [
      { text: 'Start', link: '/operations/local-development' },
      { text: 'Architecture', link: '/architecture/architecture' },
      { text: 'Operations', link: '/operations/operations' },
      { text: 'CLI docs', link: 'https://cli.ayni.build' },
    ],
    sidebar: [
      { text: 'Introduction', link: '/' },
      ...sections.map(([text, directory, names]) => ({
        text,
        items: names.map(name => ({
          text: name.split('-').map(word => word === 'ayni' ? 'Ayni' : word[0].toUpperCase() + word.slice(1)).join(' '),
          link: `/${directory}/${name}`,
        })),
      })),
      { text: 'Contributing', items: [{ text: 'Documentation', link: '/contributing/documentation' }] },
      { text: 'History', collapsed: true, items: [
        { text: 'Decisions', link: '/decisions/README' },
        { text: 'Event research', link: '/history/event-platform/' },
      ] },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/ayni-oss/orchestrator' }],
    editLink: { pattern: 'https://github.com/ayni-oss/orchestrator/edit/main/docs/:path' },
    search: { provider: 'local' },
  },
  markdown: {
    config(md) {
      // Files outside docs remain repository sources; the site links to GitHub.
      md.core.ruler.after('inline', 'repository-links', state => {
        const source = state.env.path ?? state.env.filePath
        if (!source) return
        const rewrite = (tokens: typeof state.tokens) => {
          for (const token of tokens) {
            const href = token.attrGet('href')
            if (href && !/^(?:[a-z]+:|\/|#)/i.test(href)) {
              const [file, fragment] = href.split('#')
              const absolute = path.resolve(path.dirname(source), file)
              const relative = path.relative(path.resolve('docs'), absolute)
              if (relative.startsWith('../')) {
                const repositoryPath = path.relative(process.cwd(), absolute)
                token.attrSet('href', `https://github.com/ayni-oss/orchestrator/blob/main/${repositoryPath}${fragment ? '#' + fragment : ''}`)
              } else if (file.endsWith('.csv')) {
                token.attrSet('href', '/event-platform-contracts.csv')
              }
            }
            if (token.children) rewrite(token.children)
          }
        }
        rewrite(state.tokens)
      })
    },
  },
  async buildEnd(site) {
    await fs.copyFile('docs/history/event-platform/control-plane-event-contracts.csv', path.join(site.outDir, 'event-platform-contracts.csv'))
  },
}))
