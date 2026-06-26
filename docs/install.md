# Install

## Prebuilt binaries

Download from [GitHub Releases](https://github.com/colelandolt/azure-boards-cli/releases): archives for Linux (x86_64), macOS (arm64 + x86_64), and Windows (x86_64) containing both `azure-boards` and the `ab` alias. Put them on your `PATH`.

## From source

```sh
cargo install --git https://github.com/colelandolt/azure-boards-cli
# installs both `azure-boards` and `ab`
```

Requires Rust 1.82+.

## Shell completions

```sh
# bash (~/.bashrc)
eval "$(ab completion bash)"
# zsh (~/.zshrc)
eval "$(ab completion zsh)"
# fish
ab completion fish | source
# PowerShell ($PROFILE)
ab completion powershell | Out-String | Invoke-Expression
```

## First run

```sh
ab auth login          # or rely on an existing `az login`, or set ADO_PAT
ab configure --defaults organization=<org> project=<project> team=<team>
ab context show
```
