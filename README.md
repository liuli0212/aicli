# aicli

`aicli` 是一个用 Rust 写的命令行助手：你用自然语言描述想做的事，它调用模型生成对应 shell 命令，先展示给你确认。你可以直接回车执行，也可以改完再执行。

```bash
aicli "看看当前 git 项目哪些文件很大"
```

可能生成：

```bash
git ls-tree -r -l --full-name HEAD | sort -n -k 4 -r | head -n 10
```

输出会分成三段：

```text
Explanation
  说明这条命令会做什么。

Command
----------------------------------------
$ 生成的命令
----------------------------------------

Output
----------------------------------------
命令执行结果
```

不加 `-y` 时，`aicli` 会进入可编辑确认行；加 `-y` 时会直接执行生成的命令。

## 安装

在目标机器上构建：

```bash
cargo build --release
```

安装到用户 PATH：

```bash
install -m 755 target/release/aicli ~/.local/bin/aicli
```

确认安装成功：

```bash
which aicli
aicli --help
```

当前 release 二进制使用 `rustls` 做 HTTPS，不依赖 OpenSSL。不同 CPU 或操作系统需要在对应平台重新构建。

## 配置

默认会按顺序读取：

1. `./config.toml`
2. `~/.config/aicli/config.toml`
3. `~/.aicli/config.toml`

也可以用环境变量指定：

```bash
export AICLI_CONFIG=/path/to/config.toml
```

生成配置模板：

```bash
mkdir -p ~/.config/aicli
aicli --config-template > ~/.config/aicli/config.toml
chmod 600 ~/.config/aicli/config.toml
```

配置示例：

```toml
default_provider = "gemini"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.5-flash"

[providers.deepseek]
type = "openai_compat"
api_key_env = "DEEPSEEK_API_KEY"
base_url = "https://api.deepseek.com/chat/completions"
model = "deepseek-chat"

[providers.local]
type = "openai_compat"
api_key = "local"
base_url = "http://localhost:11434/v1/chat/completions"
model = "qwen2.5-coder"
```

## Provider

支持两类 provider：

- `gemini`：Google Gemini 原生 API，默认读取 `GEMINI_API_KEY`
- `openai_compat`：OpenAI Chat Completions 兼容接口，比如 DeepSeek、OpenRouter、本地模型网关等

Gemini：

```bash
export GEMINI_API_KEY=...
aicli --provider gemini "列出最近 5 次 git commit"
```

OpenAI-compatible：

```bash
export OPENAI_COMPAT_API_KEY=...
export OPENAI_COMPAT_BASE_URL="https://api.example.com/v1/chat/completions"
export OPENAI_COMPAT_MODEL="example-chat-model"
aicli --provider openai_compat "查找当前目录最大的 10 个文件"
```

如果 provider 写在 `config.toml` 里，可以直接用 provider 名：

```bash
aicli --provider deepseek "看看当前目录有哪些 rust 文件"
```

## 常用命令

只生成，不执行：

```bash
aicli --dry-run "显示当前 git 分支"
```

生成后直接执行：

```bash
aicli -y "打印当前目录"
```

带代理执行：

```bash
https_proxy=http://127.0.0.1:7897 aicli -y "看看 top10 的大文件"
```

打开诊断日志：

```bash
https_proxy=http://127.0.0.1:7897 aicli -v "看看 top10 的大文件"
```

指定模型：

```bash
aicli --provider gemini --model gemini-2.5-flash "显示最近提交"
aicli --provider openai_compat --model qwen2.5-coder "查找大文件"
```

## 日志与排错

使用 `-v` / `--verbose` 可以把诊断信息打印到 stderr，包括：

- 配置文件路径
- provider 和 model
- 当前工作目录和 shell
- 代理环境变量
- 脱敏后的请求 endpoint
- HTTP 响应状态
- 网络错误原因链

示例：

```text
[aicli] config=/home/liuli/.config/aicli/config.toml
[aicli] provider=gemini
[aicli] env https_proxy=http://127.0.0.1:7897
[aicli] provider=gemini type=gemini model=gemini-3-flash-preview endpoint=https://...key=***
[aicli] sending model request
[aicli] timeout_secs=180
[aicli] model response status=200 OK
```

API key 会被脱敏；代理 URL 里如果带账号密码，也会被脱敏。

HTTP 请求默认超时为 180 秒。慢代理或慢模型可以调大：

```bash
AICLI_TIMEOUT_SECS=300 aicli -v "看看 top10 的大文件"
```

## 注意事项

- `aicli` 会在当前工作目录执行命令；先 `cd` 到你想操作的目录。
- 模型生成的命令不一定完美，不加 `-y` 时可以先编辑再回车。
- 对看起来会修改或删除数据的命令，`aicli` 会额外询问确认。
- 不要把 API key 写进公开仓库；推荐用 `api_key_env` 引用环境变量。
