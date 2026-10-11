// ESLint flat config. Enforces renderer import boundaries (codebase.md §3.5):
// the renderer never imports Electron main, Node built-ins, or raw `fs`.
// REQ: REQ-STUDIO-001 (renderer receives domain capabilities, never machine access).
import js from "@eslint/js";
import tseslint from "typescript-eslint";

const nodeBuiltins = [
  "fs",
  "path",
  "child_process",
  "os",
  "net",
  "http",
  "https",
  "worker_threads",
].flatMap((name) => [
  { name, message: "renderer must not use Node built-ins (ui-arch §Desktop Shell)" },
  { name: `node:${name}`, message: "renderer must not use Node built-ins" },
]);

export default [
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "electron",
              message: "renderer must not import Electron main (ui-arch §Desktop Shell)",
            },
            ...nodeBuiltins,
          ],
        },
      ],
    },
  },
  {
    files: ["electron/**/*.ts"],
    rules: {},
  },
];
