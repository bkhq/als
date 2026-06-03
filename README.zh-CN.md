# als

> English version: [`README.md`](README.md)

**一条命令发布任何东西。** 目录、单个 `.html` / `.md` 文件、mdbook
项目、源代码——`als` 自动打包、上传，给你一个公开 URL。

```bash
als auth login        # 一次性设备配对登录
als ./build           # 发布一个目录 → https://k7x2qm4j6p.a.ls
als list              # 看看自己发过哪些
als rm k7x2qm4j6p     # 撤下来
```

单个静态二进制，无运行时依赖，对接
[`api.a.ls`](https://api.a.ls)（或任何兼容的部署）。

---

## 安装

### 发布版二进制

从 [发布页][releases] 下载对应平台的压缩包，把 `als` 放到 `PATH` 里。

```bash
# Linux x64（静态 musl 链接）
curl -sSL https://github.com/bkhq/als/releases/latest/download/als-linux-x64.tar.gz \
  | tar -xz
sudo install -m 0755 ./als /usr/local/bin/als

# 其他平台:
#   als-linux-x64.tar.gz     als-linux-arm64.tar.gz
#   als-darwin-x64.tar.gz    als-darwin-arm64.tar.gz
#   als-windows-x64.zip
```

[releases]: https://github.com/bkhq/als/releases

### 从源码

需要较新的稳定版 Rust 工具链。

```bash
cargo install --git https://github.com/bkhq/als --bin als
# 或在本仓库 checkout 内:
just build && ./target/release/als --version
```

### Shell 补全

```bash
als completion bash > /etc/bash_completion.d/als
als completion zsh  > "${fpath[1]}/_als"
als completion fish > ~/.config/fish/completions/als.fish
```

---

## 第一次部署

```bash
$ als auth login

To authorize this device, visit:
  https://a.ls/verify
and enter the code:
  ABCD-EFGH

Waiting for authorization...
✓ Logged in.

$ als ./build
✓ Deployed: https://k7x2qm4j6p.a.ls
  Id:      k7x2qm4j6p
  Name:    fox042
  Version: 20260516-091500-b8e5d3
  Expires: in 7 days
```

每个站点有两个标识符：

- **`Id`** —— 10 字符 base32，服务端自动生成，**不可变**，就是 URL
  的子域名。
- **`Name`** —— kebab-case 小写标签（`a-z`、`0-9`、`-`）。可以用
  `--name my-proj` 显式指定。不指定时 CLI 自动生成 `<word><NNN>`——
  一个短英文名词加三位数字，比如 `fox042`、`mint007`。比 id 好记；
  之后可以用 `als site <id|name> --name <new>` 改名。**名字唯一性由
  服务端裁定**，被拒就换一个 `--name` 重试即可。

`als auth login` 每台机器只需要做一次，token 写到
`~/.config/als/config.toml` 后就走完。后续所有命令直接用。

在同一目录再跑 `als ./build` **会发新版本到同一个 URL**——`als`
本地记着绑定（`~/.config/als/sites/` 下每个站点一个 TOML 文件），不
用每次都传 `--name`。

---

## 我能发布什么？

`als <path>` 自动识别输入形态：

| 路径                                              | 得到什么 |
|---------------------------------------------------|----------|
| 含 `index.html` 的目录                            | 静态站点（目录直接打包上传）。 |
| 含 `book.toml` 的目录                             | 渲染后的 mdbook。 |
| 只有 `*.md` 的目录（无 `book.toml`）              | 自动 bootstrap 的 mdbook——`SUMMARY.md` 为你合成。 |
| 单个 `.html` 文件                                 | 一页静态站。 |
| 单个 `.md` 文件                                   | 单章 mdbook。 |
| `.zip` 压缩包                                     | 原样上传。 |
| 一个代码目录                                      | 只读代码 viewer（CodeMirror 6，含文件树 / 标签 / 语法高亮）。 |
| 单个源码文件                                      | 同上 viewer，一个文件。 |

需要强制时用 `--kind site` / `--kind code`。

---

## 常用技巧

### 给一个可记忆的固定 URL

```bash
als ./build --name myproj
# → https://k7x2qm4j6p.a.ls   (只要传同一个 --name，无论在哪台机器上
#   跑，URL 都稳定不变)
```

`--name` 是 kebab-case 小写：`a-z`、`0-9`、`-`，不能以 `-` 开头或结
尾。这个 name 同时是服务端的唯一性键和本地 pin 文件名，所以空格 / 标
点这些都会在解析时被拒绝。

### 给站点加密码

```bash
als ./build --pass auto              # 服务端生成一个易记密码
als ./build --pass "letmein123"      # 或者自己指定
```

已经发布过、想加 / 换密码？

```bash
als site k7x2qm4j6p --pass auto      # 轮换
als site k7x2qm4j6p --no-pass        # 清除密码（变成公开）
```

### 设置 / 延长过期时间

```bash
als ./build --expires 24h            # `5m` / `24h` / `7d` / `1y` / `never`
als ./build --expires 2026-12-31T00:00:00Z   # 绝对 RFC 3339 时间

als site k7x2qm4j6p --expires never  # 事后调整
```

### 回滚到旧版本

```bash
$ als site k7x2qm4j6p                # 查看版本历史
$ als site k7x2qm4j6p --version 20260510-103022-a3f1c2
✓ Activated version 20260510-103022-a3f1c2.
```

### 发布前本地预览

```bash
$ als preview ./docs
Preview at http://127.0.0.1:54123/
  press Ctrl+C to stop
```

预览完全离线——和上传 pipeline 用的是同一个 renderer，从临时目录通
过 `127.0.0.1` 提供服务。要暴露到局域网用 `--bind 0.0.0.0`。

### 管理你的站点

```bash
als list                             # 列出当前活跃站点 + 配额底栏
als list --state expired             # 按状态过滤
als site k7x2qm4j6p                  # 详情 + 版本历史
als rm k7x2qm4j6p                    # 删除（交互确认）
als rm --all-expired -y              # 批量删除已过期站点
```

### 脚本友好输出

```bash
als ./build --quiet                  # 只输出 URL
als ./build --json                   # 完整 JSON envelope，给 jq / 等用
```

JSON 退出码矩阵是稳定的；安全地接 `jq`。

---

## 每个项目的配置（`als.toml`）

在输入路径根下放一个 `als.toml`，调整 `als <path>` 的打包 / 渲染行
为。所有字段都可选；未知字段会硬报错，让拼写错误立刻暴露。这个文件
本身不会进 bundle。

```toml
# 站点 id —— 最高优先级的跨机器 pin。每次 deploy 成功后 CLI 会自动
# 写回这个字段，所以基本不用手动维护。一旦提交进仓库，任意克隆
# (新机器 / CI runner / 同事的笔记本) 都会部署到同一个站点，
# 完全不依赖本地 pin 缓存。10 位小写 base32 (`[a-z2-7]{10}`)。
id           = "k7x2qm4j6p"

# 项目身份。规则跟 `--name` 一样：a-z / 0-9 / `-`，不以 `-` 开头或
# 结尾。在 `id` 还没被写入之前作为兜底（比如 committed 了 name 但
# 还没跑过第一次 deploy 的情形）。
name         = "fibonacci-demo"

# 代码模式下 viewer 的页面标题。默认是目录名。
title        = "Q3 Fibonacci demo"

# 代码模式下 viewer 默认打开哪个文件。相对于输入根。如果指向的文件
# 没在 bundle 里,CLI 退出码 `code_default_file_missing`。
default_file = "src/fibonacci.ts"

# 额外的 ignore 模式,叠加在内置默认集合之上
# (.git/、node_modules/、target/、.env、*.pem、*.key……)。
# Gitignore 语法;`!pattern` 可以把默认排除掉的再加回来。
exclude      = [
    "scratch/**",
    "*.log",
    "!keep-this.log",
]
```

| 字段           | 适用模式  | 作用 |
|----------------|-----------|------|
| `id`           | 全部模式  | 站点 id —— 最强 pin。作为 `site_id` 发到服务端,服务端通过 id 识别站点。每次 deploy 成功后自动写入,只有 `--new` 会绕过。 |
| `name`         | 全部模式  | `id` 还没写入前作为兜底身份;作为 `project_name` 发到服务端。单次调用可被 `--name <n>` 覆盖。 |
| `title`        | 仅代码    | 生成的 viewer 壳的 `<title>` 和 `data-title`。 |
| `default_file` | 仅代码    | viewer 打开的第一个文件;必须在 bundle 里存在。 |
| `exclude`      | 全部模式  | 额外的 gitignore 模式。本仓没有独立的 `.alsignore` 文件,这里就是单一来源。 |

---

## 文件存哪了

```
~/.config/als/
├── config.toml                       # API URL + bearer token,按 profile 分组
└── sites/                            # 每个本地 pin 的站点一个 TOML
    ├── k7x2qm4j6p_myproj.toml        # 文件名: <id>_<name>.toml
    └── m3p4rs5t2k_fox042.toml
```

一个 pin 文件大致是：

```toml
name              = "myproj"
id                = "k7x2qm4j6p"
path              = "/home/alice/build"
url               = "https://k7x2qm4j6p.a.ls"
last_published    = "2026-05-15T19:50:00Z"
last_content_hash = "a3f1c2..."
```

文件名把不可变 id 和小写 name 合到一起,所以下划线两边任一段都能定
位 pin —— `rm ~/.config/als/sites/k7x2qm4j6p_*.toml` 就等价于 `als
unpin k7x2qm4j6p` 的手动版本。一般不用直接动这些 ——
`als auth login`、`als config set`、`als <path>` 会自动维护。

CI 场景下的环境变量覆盖：

```bash
ALS_API     # 覆盖当前 profile 的 API URL
ALS_TOKEN   # 覆盖当前 profile 的 bearer token
ALS_PROFILE # 选用具名 profile
```

---

## 默认值

| 设置            | 值                                |
|-----------------|-----------------------------------|
| API base        | `https://api.a.ls`                |
| 主页            | <https://a.ls>                    |
| LLM 速查表      | <https://a.ls/llms.txt>           |
| 单包上限        | 50 MB                             |
| 代码模式上限    | ≤ 50 文件 · 每个 ≤ 256 KB · 总计 ≤ 2 MB · 目录深度 ≤ 8 |

要换 API 用 `ALS_API`、`als config set default.api <url>`,或者通
过 `--profile <name>` 切到另一个 profile。

---

## 下一步

- [`docs/cli-spec.md`](docs/cli-spec.md) —— 每条命令、flag、退出
  码、错误标识。
- [`docs/architecture.md`](docs/architecture.md) —— 想 hack `als`
  本身就看这个 crate 图。
- [`docs/development.md`](docs/development.md) —— 本地编译 + 测试
  工作流。
- [`docs/api-reference.md`](docs/api-reference.md) —— typed
  `als-api` client 的参考,给嵌入使用。
- [`llms.txt`](llms.txt) —— Agent 友好速查表,也在
  <https://a.ls/llms.txt> 提供。

## 协议

MIT.
