# Sulafat

Cliente SSH do ecossistema **Lyra OS**, o par do Beam: enquanto o Beam conecta ao
mundo Windows via RDP, o Sulafat conecta ao mundo Linux/Unix via SSH. Funciona em qualquer
distribuição Linux moderna, com integração visual prioritária com o Lyra (GNOME/Wayland).

- Protocolo SSH via o binário `ssh` do OpenSSH, executado dentro de um terminal embutido (VTE) —
  sem implementação própria de SSH. Herda de graça chaves, `ssh-agent`, `known_hosts`,
  certificados, `ProxyJump`, multiplexação e todo o `~/.ssh/config` existente do usuário.
  Nenhuma senha ou passphrase é manuseada ou armazenada pelo Sulafat.
- Interface em GTK4 + libadwaita.
- `~/.ssh/config` é a fonte da verdade dos hosts — interoperável com o `ssh` puro. Grupo, cor e
  anotações (que não existem no `ssh_config`) ficam num TOML próprio em XDG config.

## Estrutura do repositório

- `sulafat-core`: parser/writer fiel de `~/.ssh/config`, metadados de UI, montagem de comandos e
  monitoramento de mudanças externas — sem dependência de nenhum toolkit gráfico.
- `sulafat-gtk`: frontend GTK4/libadwaita/VTE (binário `sulafat`).
- `data`: `.desktop`, metadados AppStream e ícones.
- `packaging`: artefatos para o pacote RPM no OBS (`home:rodrigosbrito:lyra/sulafat`).

## Compilando

Dependências de sistema (nomes Fedora/openSUSE): `gtk4-devel`, `libadwaita-devel`, o devel do VTE
para GTK4 (`vte-devel` no openSUSE, variante gtk4 do vte-2.91), um compilador Rust estável recente
(`cargo`, `rustc`).

```sh
cargo build --release
./target/release/sulafat
```

Variável de ambiente `SULAFAT_LOG` controla o nível de log (`tracing-subscriber`), por exemplo
`SULAFAT_LOG=debug ./target/release/sulafat`. O conteúdo das sessões de terminal nunca é
registrado nos logs.

## Validação

O mesmo contrato executado em cada push para `main` e em cada pull request pode ser reproduzido
localmente com:

```sh
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features --offline -- -D warnings
python3 scripts/check-i18n.py
rpmspec -P packaging/sulafat.spec >/dev/null
desktop-file-validate data/org.lyraos.Sulafat.desktop
appstream-util validate-relax --nonet data/org.lyraos.Sulafat.metainfo.xml
```

A suíte cobre explicitamente recusa de symlinks, permissões restritas, escrita e substituição
atômicas, rotação/restauração de backups e conflitos causados por mudanças externas no
`~/.ssh/config`. Os testes de precedência exigem o cliente OpenSSH (`openssh-clients`
ou `openssh-client`, conforme a distribuição): comparam `ssh -G -F` antes e depois de
salvar fixtures temporárias, sem abrir conexões. Cobrem `Include`, campos repetidos,
`Match`, comentários, CRLF e ausência de quebra de linha no fim do arquivo.

Ao editar um host existente, campos sem mudanças mantêm seus bytes e posições.
As opções avançadas preservadas ficam entre os mesmos campos conhecidos; linhas
alteradas ocupam os espaços antigos, e linhas adicionais ficam antes da próxima
linha avançada preservada ou depois do último espaço substituído. Ao acrescentar
opções no fim, elas entram depois da última linha avançada existente (ou no fim do
bloco se não havia nenhuma). Esse editor não representa a posição de uma opção
avançada em relação aos campos do formulário: para mover deliberadamente um
`Include` através de um campo conhecido, edite o arquivo diretamente.

O formulário mostra os campos locais do bloco, não o resultado de resolver `Include`.
Portanto, alterar um campo posterior a um `Include` não força esse valor a prevalecer
sobre o arquivo incluído. Uma porta `22` explícita é preservada ao salvar sem mudanças.

## Uso

- Gerencie hosts a partir dos blocos `Host` do seu `~/.ssh/config`: busca, grupos e cor por
  ambiente.
- Duplo clique/Enter em um host conecta numa nova aba; conexão rápida `usuário@host` conecta sem
  criar um perfil.
- Prompts do OpenSSH (senha, confirmação de fingerprint de host novo/alterado) aparecem no próprio
  terminal, como no `ssh` puro.
- Botão "Abrir arquivos" abre o local `sftp://` correspondente no gerenciador de arquivos padrão.

## Limitações conhecidas (v1)

- Sem SFTP gráfico embutido, gravação de sessão, broadcast de comandos ou snippets.
- Sem mosh, telnet ou serial.
- Arquivos referenciados por `Include` (e `/etc/ssh/ssh_config`) são somente leitura.
- Hosts com múltiplos padrões ou wildcard (`Host web1 web2`, `Host *`) são somente leitura.
- Sem gerenciamento de chaves (gerar/copiar) — planejado para uma v1.x.

Essas limitações são decisões deliberadas de escopo para a v1, não bugs.

## Licença

GPL-3.0-or-later. Veja [`LICENSE`](LICENSE).
