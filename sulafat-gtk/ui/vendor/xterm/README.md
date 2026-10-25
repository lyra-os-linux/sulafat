# xterm.js (vendorizado)

Cópia sem modificações dos módulos ESM publicados no npm, sem os comentários
`sourceMappingURL`. Ficam no repositório porque a interface é embutida no
binário (CSP `'self'`) e o build do OBS é offline.

| Pacote | Versão | Arquivo |
| --- | --- | --- |
| `@xterm/xterm` | 6.0.0 | `xterm.mjs`, `xterm.css` |
| `@xterm/addon-fit` | 0.11.0 | `addon-fit.mjs` |
| `@xterm/addon-web-links` | 0.12.0 | `addon-web-links.mjs` |

Licença MIT, em `LICENSE` (a mesma dos addons, publicados pelos mesmos autores).

Para atualizar: `npm pack` das três versões, copiar `lib/*.mjs` e
`css/xterm.css`, remover a linha `//# sourceMappingURL` e testar a sessão.
