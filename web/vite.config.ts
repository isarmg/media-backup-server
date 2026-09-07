import { createSarmgReactViteConfig } from "@sarmg/web-toolchain/vite";
import { mergeConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export default mergeConfig(createSarmgReactViteConfig({ base: "/admin/" }), {
  resolve: { alias: [{ find: /^@sarmg\/admin-ui$/, replacement: fileURLToPath(new URL("./shell/ui.js", import.meta.url)) }] },
  plugins: [{
    name: "media-font-license",
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "assets/MapleMono-OFL.txt", source: readFileSync(new URL(import.meta.resolve("@sarmg/web-fonts/OFL.txt"))) });
      this.emitFile({ type: "asset", fileName: "assets/CJK-LICENSE.txt", source: readFileSync(new URL("./fonts/CJK-LICENSE.txt", import.meta.url)) });
    },
  } satisfies Plugin],
  build: {
    rollupOptions: {
      output: {
        entryFileNames: "assets/admin.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: (assetInfo: { names: string[] }) =>
          assetInfo.names.some((name) => name.endsWith(".css"))
            ? "assets/admin.css"
            : "assets/[name][extname]",
      },
    },
  },
  server: {
    port: 5174,
    proxy: { "/api/v2": "http://127.0.0.1:8080" },
  },
});
