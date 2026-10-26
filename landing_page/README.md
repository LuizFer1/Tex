# Landing page do Tex

Página estática, em português (Brasil), pronta para publicar. Abre `index.html` no navegador. Não tem build, formulário, analytics, cookie nem chat. O navegador só guarda o tema (`localStorage`, chave `tex-theme`) e o idioma (`tex-lang`, `pt` ou `en`). Sem tema salvo, a página segue o tema do sistema. O botão do cabeçalho mostra `PT` ou `EN` conforme o idioma atual; um clique alterna. O aviso de copyright do rodapé fica igual nas duas línguas.

## Publicar na Vercel

No projeto importado deste repositório, defina o diretório raiz como `landing_page`. Framework: Other. Comando de build vazio. A Vercel serve o `index.html` desta pasta e ignora o programa em Rust.

## Arquivos

| Arquivo | O que é |
|---|---|
| `index.html` | A página. HTML, CSS e o troca-tema. |
| `assets/` | Ícone e os três mascotes. Fundo amarelo opaco, parte da marca. |
| `fonts/` | Geist, Geist Mono, Nunito e IBM Plex Mono em `.woff2`, com as licenças SIL OFL. |

Geist e Geist Mono são a página. Nunito e IBM Plex Mono só aparecem dentro das janelas desenhadas do app.

## Links

| Rótulo | Destino |
|---|---|
| Baixar para Windows | https://github.com/LuizFer1/Tex/releases |
| Contribua no GitHub, GitHub | https://github.com/LuizFer1/Tex |
| Pague-me um café, Ko-fi | https://ko-fi.com/fernandodantasdev |
| Versões | https://github.com/LuizFer1/Tex/releases |
| PolyForm Noncommercial 1.0.0 | https://polyformproject.org/licenses/noncommercial/1.0.0 |

O aviso do rodapé fica literal: `Required Notice: Copyright (c) 2026 Luiz Fernando (https://github.com/LuizFer1/Tex)`.
