import {readFileSync} from "node:fs";
export function foundationFontLicenses(prefix = "assets/") {
  const directory = new URL("./", import.meta.resolve("@xcss/web-fonts/OFL.txt"));
  return {
    name: "foundation-font-licenses",
    generateBundle() {
      for (const name of ["OFL.txt", "CJK-LICENSE.txt", "NORMAL-LICENSE.txt"]) {
        this.emitFile({type: "asset", fileName: prefix + name, source: readFileSync(new URL(name, directory))});
      }
    },
  };
}
