# WinCache Cleaner 2.0

Limpador **somente de cache** para Windows 10 e 11, escrito em Rust com interface gráfica `egui/eframe`. Projetado para ser simples de usar e conservador: não possui opção para apagar histórico, cookies, credenciais, downloads, registro nem dados de aplicativos.

![Ícone do WinCache Cleaner](assets/icon-preview.png)

## O que mudou na versão 2.0

- Interface profissional em tema escuro, com navegação lateral, cartões de resumo, seleção individual de 10 categorias e barras de comparação.
- **Análise antes da limpeza**; a ação de exclusão só é habilitada após uma análise com arquivos elegíveis.
- Limpeza e análise em **thread separada**, com progresso e interface responsiva.
- Tela **Último relatório**, mostrando bytes removidos, arquivos removidos e falhas por categoria.
- Ícone `.ico` incorporado ao executável Windows por meio do script `build.rs`.
- Abrange caches adicionais e inclui o caminho `Network\\Cache` usado pelo Edge.
- Proteção contra symlinks/junctions, arquivos recentes e pastas que não são cache.

## Categorias de limpeza

| Categoria | Pastas / arquivos processados | Política |
|---|---|---|
| Temporários do usuário | `%LOCALAPPDATA%\Temp` | Apenas arquivos com **48 h ou mais** |
| Temporários do Windows | `%WINDIR%\Temp` | Apenas arquivos com **48 h ou mais** |
| Shaders DirectX | `%LOCALAPPDATA%\D3DSCache` | Arquivos com **24 h ou mais** |
| Shaders NVIDIA | `%LOCALAPPDATA%\NVIDIA\DXCache`, `GLCache` e `%PROGRAMDATA%\NVIDIA Corporation\NV_Cache` | Arquivos com **24 h ou mais**, se existirem |
| Miniaturas do Explorador | `%LOCALAPPDATA%\Microsoft\Windows\Explorer\thumbcache_*.db` | Arquivos com **24 h ou mais**; só esses nomes |
| Ícones do Explorador | `%LOCALAPPDATA%\Microsoft\Windows\Explorer\iconcache_*.db` | Arquivos com **24 h ou mais**; só esses nomes |
| Páginas/imagens do Edge | `<perfil>\Cache`, `<perfil>\Network\Cache` | Edge fechado |
| Código do Edge | `<perfil>\Code Cache` | Edge fechado |
| Gráficos do Edge | `<perfil>\GPUCache`, `DawnCache`, `GrShaderCache`, `ShaderCache` e caches de shaders na raiz de `User Data` | Edge fechado |
| Mídia do Edge | `<perfil>\Media Cache` | Edge fechado |

O programa analisa os perfis `Default`, `Profile N` e `Guest Profile` no diretório padrão `%LOCALAPPDATA%\Microsoft\Edge\User Data`.

**Não remove:** histórico, cookies, favoritos, senhas, logins, sessões, downloads, dados de autofill, armazenamento local de sites, IndexedDB, diretórios de `Service Worker`, `CacheStorage`, dados de WebView2, Lixeira, Prefetch, Windows Update, WinSxS, arquivos de crash dump, backups ou registro do Windows.

### Importante sobre a limpeza

- A exclusão é **permanente**. Revise as categorias antes de confirmar.
- A análise é uma estimativa de **tamanho lógico**, não uma medição de espaço físico livre. Arquivos podem ser alterados ou bloqueados entre analisar e limpar.
- A limpeza não encerra processos, reinicia o Windows ou modifica serviços. Arquivos em uso e sem permissão ficam intactos.
- Se `msedge.exe` estiver em execução (inclusive em segundo plano pelo *Startup Boost*), todas as categorias de Edge são ignoradas. Feche o navegador completamente e execute a análise novamente.
- Shaders e caches do Windows podem ser recriados, ocasionando lentidão temporária no primeiro uso de alguns aplicativos após a limpeza.
- O cache do Explorador pode estar bloqueado enquanto o Windows está em execução. O programa **não** força o fechamento do Explorador.
- Pastas de cache personalizadas configuradas em políticas do Edge não são detectadas; o programa não percorre discos em busca delas.
- Algumas categorias podem exigir permissões adicionais (por exemplo `%WINDIR%\Temp`). O programa não solicita elevação automática e ignora arquivos inacessíveis.

## Como compilar para Windows

1. Instale o Rust: <https://rustup.rs/>. Utilize a toolchain **x86_64-pc-windows-msvc**.
2. Instale o **Visual Studio Build Tools** e marque o componente **Desenvolvimento para desktop com C++**.
3. Extraia o ZIP do projeto em uma pasta local.
4. Abra o PowerShell nessa pasta e execute:

```powershell
cargo test
cargo build --release
```

O executável será gerado em:

```text
target\release\wincache-cleaner.exe
```

Também é possível abrir a interface diretamente com `cargo run --release`.

**Observação:** este pacote inclui o código-fonte, não um `.exe` previamente compilado. A execução em Windows deve ser validada pelo usuário ou pela automação do repositório.

## Compilar automaticamente com GitHub Actions

Este projeto contém `.github/workflows/windows-build.yml`. Se você publicar a pasta em um repositório no GitHub, abra **Actions → Compilar no Windows → Run workflow**. Após uma compilação bem-sucedida, o executável aparecerá no artefato **WinCache-Cleaner-Windows**. O workflow executa `cargo test` antes de `cargo build --release`.

## Estrutura

```text
wincache-cleaner/
├── Cargo.toml
├── build.rs
├── README.md
├── assets/
│   ├── wincache.ico
│   └── icon-preview.png
├── .github/workflows/
│   └── windows-build.yml
└── src/
    └── main.rs
```

## Princípios de segurança

1. Os caminhos de limpeza são **fechados e predefinidos**, derivados de variáveis comuns do Windows. O programa não permite fornecer pastas arbitrárias para exclusão.
2. Só tenta remover **arquivos regulares**. Não remove as pastas raiz nem segue links simbólicos e junctions; recusa raízes com ancestrais desse tipo.
3. TEMP utiliza limite de 48 horas; caches de shader e Explorador, de 24 horas. Cache do Edge só é processado com o Edge fechado.
4. Para o Explorador, somente os padrões exatos `thumbcache_*.db` e `iconcache_*.db` são elegíveis.
5. O programa não força a exclusão de arquivos bloqueados, não mexe em serviços do sistema e não limpa dados pessoais.

## Limitações conhecidas

- Sem limpeza agendada, sem limpeza do Edge em execução e sem uso de APIs específicas de cache para limpeza transacional.
- Se o Edge abrir novamente durante a limpeza, arquivos em uso provavelmente serão bloqueados, mas evite reabrir o Edge até a conclusão.
- O programa não detecta locais de cache personalizados fora dos caminhos padronizados.
- Em PCs gerenciados por empresas, recomenda-se confirmar a política de retenção de dados e caches antes da execução.

## Licença

MIT (consulte `LICENSE`).
