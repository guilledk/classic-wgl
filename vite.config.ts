import { defineConfig } from 'vitest/config';
import { fileURLToPath, URL } from 'node:url';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import type { Plugin } from 'vite';

const shadersDir = fileURLToPath(new URL('./src/shaders', import.meta.url));

// Shaders are source code and live in src/shaders, but the engine fetches
// them at runtime through /shaders/* urls (see public/manifest.json).
// This plugin serves them from src during dev and copies them into
// dist/shaders on build.
function shaderAssets(): Plugin {
  return {
    name: 'shader-assets',

    // dev: serve /shaders/* straight from src/shaders
    configureServer(server) {
      server.middlewares.use('/shaders', (req, res, next) => {
        const url = req.url ?? '';
        const name = path.normalize(url.split('?')[0]).replace(/^[/\\]+/, '');
        const file = path.join(shadersDir, name);
        if (!file.startsWith(shadersDir)) {
          next();
          return;
        }
        try {
          const source = readFileSync(file);
          res.setHeader('Content-Type', 'text/plain');
          res.end(source);
        } catch {
          next();
        }
      });
    },

    // build: copy src/shaders/* into dist/shaders/*
    generateBundle() {
      for (const file of readdirSync(shadersDir))
        this.emitFile({
          type: 'asset',
          fileName: `shaders/${file}`,
          source: readFileSync(path.join(shadersDir, file), 'utf-8'),
        });
    },
  };
}

export default defineConfig({
  root: '.', // your project root
  base: '/',
  server: {
    port: 5173,
    open: true,
  },
  plugins: [shaderAssets()],
  resolve: {
    alias: {
      '/classic': fileURLToPath(new URL('./src/classic', import.meta.url)),
      '/lib': fileURLToPath(new URL('./src/lib', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'html'],
      skipFull: false,
      include: [
        'src/lib/**/*.ts',
        'src/classic/ecs.ts',
        'src/classic/camera.ts',
        'src/classic/collision.ts',
        'src/classic/utils.ts',
        'src/classic/registry.ts',
      ],
    },
  },
});
