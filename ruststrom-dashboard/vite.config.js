import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    // The dev server injects the admin token, so keep it local by default
    host: process.env.VITE_HOST || '127.0.0.1',
    port: 8084,
    proxy: {
      '/api': {
        target: process.env.VITE_API_URL || "http://127.0.0.1:9091",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ''),
        // The admin token is added server side and never exposed to the browser
        headers: process.env.RUSTSTROM_ADMIN_TOKEN
          ? { Authorization: `Bearer ${process.env.RUSTSTROM_ADMIN_TOKEN}` }
          : {}
      }
    }
  },
  build: {
    outDir: 'dist',
    assetsDir: 'assets'
  }
})
