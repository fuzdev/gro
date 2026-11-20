---
'@fuzdev/gro': minor
---

feat: add the tsv host module, `@fuzdev/gro/tsv.ts`, grant-free: tsv's formatters (`format_typescript`, `format_svelte`, `format_css`), its import parse (`parse_imports_typescript`, `parse_imports_svelte`), and `create_ignore_stack`, a frozen ignore stack in place of the binding's `IgnoreStack` class, with types of its own (`TsvFormatResult`, `TsvImportsResult`, `TsvError`, `TsvIgnoreStack`, and the options); the Node implementation wraps `@fuzdev/tsv`, reading options as data, so a proxy or a getter is refused, and Gro's own modules import it, never `@fuzdev/tsv`, but for the Node loader's type strip
