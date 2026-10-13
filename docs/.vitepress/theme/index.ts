import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import AyniBrand from './AyniBrand.vue'
import Layout from './Layout.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout,
  async enhanceApp({ app }) {
    if (!import.meta.env.SSR) {
      const [{ default: mermaid }, { default: elkLayouts }] = await Promise.all([
        import('mermaid'),
        import('@mermaid-js/layout-elk'),
      ])
      mermaid.registerLayoutLoaders(elkLayouts)
    }
    app.component('AyniBrand', AyniBrand)
  },
} satisfies Theme
