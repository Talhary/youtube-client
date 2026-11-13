import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The app serves these files to an Android WebView from https://app.streampipe/, so paths
// are relative and the output targets older WebView versions.
export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    target: ['chrome80'],
    cssTarget: ['chrome80'],
    outDir: 'dist',
    assetsInlineLimit: 0,
  },
});
