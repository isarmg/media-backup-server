import { createSarmgReactViteConfig } from "@sarmg/web-toolchain/vite";
import { mergeConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";

export default mergeConfig(createSarmgReactViteConfig({ base: "/admin/" }), {
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
