import adapter from "@sveltejs/adapter-node";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({ out: "build" }),
    csp: {
      mode: "nonce",
      directives: {
        "default-src": ["self"],
        "base-uri": ["self"],
        "connect-src": ["self"],
        "form-action": ["self", "https://github.com"],
        "frame-ancestors": ["none"],
        "img-src": ["self"],
        "object-src": ["none"],
        "script-src": ["self"],
        "style-src": ["self"],
      },
    },
  },
};

export default config;
