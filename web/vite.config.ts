import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    // in development the Rust server runs on :8080; Vite forwards /api to it
    proxy: { '/api': 'http://localhost:8080' },
  },
  build: {
    target: 'es2022',
    // React and the app change at different rates: separate chunks cache better
    rollupOptions: {
      output: { manualChunks: (id: string) => (/node_modules[\/](react|react-dom|scheduler)[\/]/.test(id) ? 'react' : undefined) },
    },
  },
  test: {
    environment: 'jsdom',
  },
})
