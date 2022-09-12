import { resolve } from 'path'
import { defineConfig } from "vite";
import { visualizer } from "rollup-plugin-visualizer";
import vue from "@vitejs/plugin-vue";
import vueJsx from "@vitejs/plugin-vue-jsx";
import VitePluginLinaria from "vite-plugin-linaria";

// https://vitejs.dev/config/
export default defineConfig({
  server: {
    port: 3000,
  },
  plugins: [
    vue(),
    vueJsx(),
    VitePluginLinaria(),
    visualizer(),
  ],
  build: {
    chunkSizeWarningLimit: 1024 * 1024,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        splashscreen: resolve(__dirname, "splashscreen.html"),
      },
      output: {
        manualChunks: {
          common: [
            "dayjs",
            "localforage",
            "debug",
            "lodash-es",
            "pretty-bytes",
            "ulid",
            "bluebird",
            "js-base64",
            "pretty-bytes",
            "codemirror",
            "crypto-js",
          ],
          ui: [
            "vue",
            "vue-router",
            "vue-i18n",
            "pinia",
          ],
          naive: [
            "naive-ui",
          ]
        },
      },
    },
  },
});
