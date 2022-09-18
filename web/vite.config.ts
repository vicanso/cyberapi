import { resolve } from 'path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueJsx from "@vitejs/plugin-vue-jsx";
import VitePluginLinaria from "vite-plugin-linaria";
import svgLoader from "vite-svg-loader";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    vueJsx(),
    VitePluginLinaria(),
    svgLoader(),
  ],
  build: {
    chunkSizeWarningLimit: 1024 * 1024,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
      },
      output: {
        manualChunks: {
          ui: [
            "vue",
            "vue-router",
          ],
          naive: [
            "naive-ui",
          ]
        },
      },
    },
  },
  server: {
    proxy: {
      "/api": {
        target: "http://localhost:3000",
      },
    },
  },
})
