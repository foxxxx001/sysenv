# sys
[English](./README.en.md) | **简体中文**
跨平台（Windows / Ubuntu）Http接口客户端工具、AI模型查询工具、系统 PATH 与环境变量管理工具**，单一二进制、子命令划分功能。内置六大能力：



* **PATH 管理**：增删查改系统 PATH，自动持久化并即时作用于当前环境

* **注册表导入导出**：PATH 一键导出为注册表 / JSON / 文本，可反向合并或整体替换

* **链接到 PATH**：把任意文件挂进 PATH 目录，之后任意目录直接敲名字即可执行

* **环境变量管理**：读写系统环境变量，可选择持久化或仅临时生效

* **AI 模型查询**：从 models.dev 按名称爬取模型 / Provider 信息

* **httpie 兼容 HTTP 客户端**：参数与 httpie 保持一致

* **快捷命令垫片**：一键生成 `spath / senv / slink / shttp / sai / stask / sfile` 短命令

* **日志查看**：`sys log` 彩色 less 式日志查看器——解析 Nginx / Linux / Windows / Log4j 等常见日志，时间橙色、级别渐变（info 及以下绿、warning 起红）、app 深黄；`--level` 过滤、`--search` 搜索（n/u 跳转）、`--line` / `--newer` 定点、`-f` 追踪文件



```
sys (Made by Gary-china)

Usage: sys <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import / unlock
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query AI model info (DataLearner), provider info (models.dev), inspect the config (info), chat with configured providers
  task   Query and kill processes (list PID/name/path; kill by PID or name)
  short  Install shell shims for every subcommand (spath/senv/slink/shttp/sai/stask/sfile)
  data   Query / modify / convert structured data (JSON/JSONL/YAML/TOML/CSV/MD/TABLE), dasel-compatible (alias dasel)
  log    Colored less-style log viewer: parse timestamps/levels/app markers, filters, search, -f follow
```



***

## 特性总览



| 功能        | 子命令                  | 核心特性                                                                                   |
| --------- | -------------------- | -------------------------------------------------------------------------------------- |
| PATH 增删查改 | `path`               | add 默认仅当前会话（`--save` 才持久化）；remove 可 `--temporary`；自动转绝对路径与去重；前置插入；user/machine 双作用域；`path unlock`（Windows）检测并解锁被其他进程占用的文件/目录                    |
| 注册表导入导出   | `path export/import` | `.reg` / JSON / TXT 三种格式；合并或 `--replace` 整体替换；跨机迁移备份                                   |
| 链接到 PATH  | `link`               | 硬链接 → 符号链接 → 拷贝三级自动回退；Windows `.cmd` 垫片；自定义命令名；系统目录或托管目录                               |
| 环境变量      | `env`                | get /set/unset/list；默认持久化；`--temporary` 仅当前 shell；machine 作用域                          |
| AI 模型查询   | `ai`                 | `ai model`（models.dev：226 个 Provider、8000+ 模型；24h 缓存；canonical 优选；无参列出全部；`--date` / `--open` / `--model-type`（支持逗号多值取交集）/ `--search / --list / --json / --refresh`）；`ai cn-model`（datalearner：1039 个中文模型；`--date` / `--model-type` 过滤；精确命中附带详情页上下文长度、模态与**参数规模（参数量）**）；`ai info`（本地配置查询：`info provider` 列出 name/api_base/api_key/docs/console/sample（sample 为该 provider models 接口的示例请求，如 `https://openrouter.ai/api/v1/models  Authorization:"Bearer sk-sample"`）、`info model` 列出 `{provider}:{name}` 模型清单、`info sale-price` 抓官网定价页、`info list-model` / `info models` 用 provider 的 models 接口查可用模型（同 `sys ai chat --query-model`）；`info providers/models` 支持 `--free` 显示内置免费路由）；`ai price`（models.dev 价格，`--src openrouter` 换 openrouter.ai 目录，见命令参考）；`ai balance`（独立子命令，minimax / agnes / alibaba-cn / deepseek / tencent 用 api_key 查官方余额，其余给控制台地址）；`ai config`（导出配置为 codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder 格式，支持 `-m` 多模型选择（与 chat 同格式）、`--show` 预览目标配置文件地址与内容、`-f` 写文件且写入前自动备份、`--update` 就地合并更新默认配置文件且先备份、`--import` 从 codebuddy / workbuddy / opencode / zcoder 配置文件反向导入 Provider/模型到本地配置且先备份（新增优先、本地已有同端点 Provider 保留本地；**以 api_base 判定同一 Provider**——同端点的条目归并为同一块、不写成多个）；导出（codex/opencode/litellm/freellmapi/codebuddy/workbuddy/trae/zcoder）同样按 api_base 归并，不建立多个条目、`--src` 指定 agent 配置目录（非默认安装位置时导入/导出/更新/列表均生效）、`--list` 只列出默认配置文件实际存在的格式）；`ai chat`（多 Provider 动态加权轮询聊天，`-m` 覆盖模型、`--list-model / --list-provider`，请求前按配置的 max_input_tokens 截断超长消息，能力字段默认只读本地配置、不查询 models.dev，`--update` 显式补齐缺失的 max_input_tokens / type 并写回配置，`--free[=ROUTER]` 内置免费路由 kilo-auto/blockrun，OpenAI / Anthropic 兼容）；`ai server`（`sys ai server [ADDR]` 以 OpenAI 兼容 API 对外提供 `/v1/chat/completions` 与 `/v1/models` 服务（默认 127.0.0.1:10000，可只给端口/IP；`--help` 查询服务状态并打印配置的 provider/model））；`ai image`（OpenAI 兼容 Images API `POST /v1/images/generations` 生成图片：`-o` 保存目录 / `-n` 数量 / `-s` 尺寸 / `--url` 下载 URL 版；b64 解码保存，文件头识别 png/jpg/gif/webp，多模型加权轮询与权重奖惩同 chat） |
| 进程管理      | `task`               | 无参列出全部进程；查询（PID / 名称 / 路径，名称模糊匹配）；`-o` 查端口占用进程；按 PID 或名称终止；`-f` 强制 |
| 文本搜索替换  | `file`               | fd/sd 风格：1 参搜 stdin（带扩展名如 `me.txt` 则显示文件，引号包裹强制搜索）；2 参目录树文本/源码搜索（`-e` / `-i` / `-t` / `-w` / `-c` / `-n` 行号）；3 参 OLD NEW PATH 就地替换；`-l` 统计行数（文件 / stdin / 匹配行）；`-S` 大小 / `--newer` / `--older` 时间 / `-d` 深度筛选，无 PATTERN 时列文件；`-md5 FILE [MD5]` / `-sha FILE [SHA1]` 哈希与比对；`--enc SRC [DST]` 编码转换（gbk/gb2312 ↔ utf-8，默认转 utf-8，`--from` 指定源编码、缺省自动检测，DST 缺省输出 stdout） |
| 数据查询转换  | `data` / `dasel`     | dasel 兼容**查询 / 修改 / 转换**结构化数据，七格式互转（JSON / JSONL / YAML / TOML / CSV / MD / TABLE）：选择器（`a.b[0]` / `list[*]` / `m["中文键"]` / 根数组，含 **Python 式切片** `[:2]` `[1:3]` `[-2:]` `[::2]` `[::-1]`）、赋值（`= VALUE` / `= null` 删除 / `= @FILE` / `= strenv(NAME)` / `+= -= *= /=`）、函数（`filter` / `remove` / `search` / 字符串函数 / **`count()` 计数**）、谓词裸过滤（csv/table/jsonl 自动过滤行数组）、**jq 式键投影** `data[*]\|{id,name}`（**裸 `{k1,k2}` 直接投影根文档**：对象根只保留这些键，如 `cat 4k.json \| sys data '{wallpaper,warningText}'`；数组根逐行投影，同 `[*]\|{...}`；**深层节点管道可省**：`sites[0]{key,name}` / `sites[0].{key,name}` 等价 `sites[0]\|{key,name}`）、`--depth` 修剪、`--limit` 取前 N 行、`--header`/`--no-header` 表头控制、`-w` 写回、`-u` URL 拉取、GBK/GB18030 等**非 UTF-8 自动解码**、tab 缩进 YAML 归一、TTY 下 json/yaml jq 式上色；**完整说明与示例见第 11 节** |
| HTTP 客户端  | `http`               | httpie 参数子集对齐；**默认 application/json**（`-f`/`--multipart`/显式头可覆盖）；JSON / 表单 /multipart/ 原始体；嵌套 JSON；下载 / 重定向 / 认证 / 离线模式；`--help` 参数说明与示例；`--debug` 打印实际请求与响应（含头） |
| 网络探测      | `net`                | 从文件批量读取 http(s) 地址并探测：`-i` 读文件（`#` 注释行跳过、每行多个地址、地址截取到空格/标点）；`-t domain`（默认，连续 3 次 GET 取平均响应时间，全部测完按平均时间升序打印）/ `-t tv`（响应体须为 JSON，支持 `--sort` 统计排序；旧名 `json`）/ `-t tvs`（兼容 tv 与聚合两种 JSON：单仓 `.sites` / 多仓 `.urls` 分组输出）/ `-t file`（响应体须为普通文本）/ `-t test`（只探测可访问性，不可访问的屏幕打印错误并从源文件删除，`-o` 输出清理后的源文件行）/ `-t info`（用 `-n` 域名查 RDAP 注册信息，查不到逐层回退上层域名）；tv/file 模式每测完一个即打印 url/可用性/状态码/响应时间/格式，`-o` 把检测成功的 url 写入文件（一行一个）；`--timeout` 单次请求超时 |
| 日志查看      | `log`                | 彩色 less 式日志查看器：解析 Nginx / Linux / Windows / Log4j 等常见日志——**时间**（ISO/斜杠/nginx access/syslog/US 多种格式，行首或行中）橙色显示；**级别**（trace→info 渐浅绿、warning→error→fatal 渐深红）渐变着色；**app 标志**深黄；`--level` 按级别过滤（warn==warning、err==error）；`--search` 关键字搜索（行号 + 反显，交互下 n/u 循环跳转）；`--line N` 从第 N 行后解析（保留原始行号）；`--newer TIME` 只显示该时间之后的日志；`-f` 追踪文件变化滚动展示（150ms 轮询，截断/轮转自动重读）；TTY 下 less 式交互（PageUp/PageDown/↑/↓/Home/End/g/G/q），管道输出纯文本无 ANSI |
| 快捷垫片      | `short`              | 一键安装七种短命令（spath/senv/slink/shttp/sai/stask/sfile）；Windows `.cmd` / Linux sh 脚本；自动加入 PATH                                       |



***

## 构建

需要 Rust 1.79+（`rustup` / `cargo`）。



```
cargo build --release
# 产物: target/release/sys(.exe)
```

### 发布产物



* 发布产物统一 **UPX 压缩** 后放入 `dist/` 目录

* 文件名包含平台 + 架构 + 版本号：`sys-<平台>-<架构>_v<版本>`


  * Windows：`sys-windows-x86_64_v0.2.8.exe`

  * Ubuntu：`sys-linux-x86_64_v0.2.8`

* 一键发布脚本（推荐）：**每次编译成功后自动清除临时编译产物**（`target/`），仅保留 `dist/` 发布产物

  * Windows：`powershell -File build.ps1`（构建 → UPX 压缩 → 冒烟 → 自动清理）

  * Ubuntu：`./build.sh`（同流程；系统无 UPX 时自动下载静态版本）

* 手动流程（示例，Windows）：



```
cargo build --release
upx --best -o dist/sys-windows-x86_64_v0.2.8.exe target/release/sys.exe
cargo clean
```



* 版本号变更与新增 / 修复内容记录在 `patch.md`

## 平台与持久化机制



* **Windows**：默认写用户作用域注册表 `HKCU\Environment`（新进程自动继承）；`--scope machine` 写 `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`，需管理员。

* **Ubuntu**：默认写入 `~/.config/sys/` 托管文件，并自动在 shell 启动脚本中注入，使持久化变量 / 路径对新 shell 生效；machine 作用域写 `/etc/environment`，需 root。

* 无论哪种平台，`path add --save`、`env set` 都会**持久化并同时更新当前进程环境**（Linux 下向父 shell 输出可粘贴的 export 片段，Windows 下注册表广播），保证 "持久化 + 当前生效" 双满足。

* `path add` 默认**仅当前会话**：更新当前进程并打印可粘贴的 shell 片段（`export` / `$env:`），不写入任何持久化存储；加 `--save` 才持久化。其余写操作（`path remove`、`env set/unset`、`link`、`short`）默认持久化，加 `--temporary` 仅当前会话。



***

## 1. PATH 管理（`path`）

### 特性



* `list`：列出当前生效的完整 PATH（合并了进程与持久化作用域）

* `add`：追加到末尾；`-p/--prepend` 插入到开头；**默认仅当前会话**（打印可粘贴片段，不持久化），`--save` 持久化供新进程继承；自动转绝对路径、大小写不敏感去重、跳过空条目

* `remove`：按绝对路径匹配移除（任意作用域）

* `has`：同时检查当前进程与已持久化作用域，报告该目录是否在 PATH 中

* `export / import`：注册表文件（`.reg`）、JSON（`.json`）、纯文本（`.txt`）三种格式双向转换

* `--scope user|machine`：控制 **`--save`** 的写入作用域

* `--save`：持久化（默认 user，`--scope machine` 需管理员/root）；不加则仅当前会话

### 示例



```
sys path list                      # 列出当前生效 PATH
sys path add D:\tools              # 仅当前会话（打印可粘贴片段，不持久化）
sys path add D:\tools -p           # 前置（仍仅当前会话）
sys path add D:\tools --save       # 持久化（user 作用域），新进程自动继承
sys path add D:\tools --save --scope machine   # 机器级持久化（需管理员/root）
sys path remove D:\tools
sys path has D:\tools              # 同时检查当前进程与已持久化作用域
sys path unlock D:\work\a.txt      # Windows：检查文件/目录是否被进程占用，列出占用进程（PID/名称/路径）并关闭其句柄解锁
```

## 2. PATH 导入 / 导出（`path export/import`）

### 特性



* `export` 按目标扩展名自动选择格式：`.reg`（Windows Registry Editor 格式，可双击导入）、`.json`（结构化，含作用域信息）、`.txt`（一行一个条目）

* `import` 默认**合并**（按绝对路径去重后追加）；`--replace` 整体替换

* 适合跨机器迁移、备份还原、批量修改前快照

### 示例



```
sys path export backup.reg         # Windows Registry Editor 格式，可双击导入
sys path export paths.json
sys path import backup.reg         # 合并进用户 PATH（去重）
sys path import backup.reg --replace   # 整体替换
sys path export paths.txt          # Ubuntu 下交换备份
```

## 3. 链接到 PATH（`link`）

### 特性



* 把任意路径下的文件放进一个 **PATH 目录**，之后任意目录直接敲名字执行

* 链接方式自动尝试：**硬链接 → 符号链接 → 拷贝**（跨卷 / 无权限时逐级自动回退），Windows 下需要开发者模式时自动降级拷贝

* Windows 对非 `.exe` 目标（`.py` / `.cmd` / 脚本等）自动生成同名 `.cmd` 垫片，保证 cmd / PowerShell 都能直接调用

* `--name` 自定义命令名；`--method hard|symlink|copy` 强制指定方式

* `--dir` 指定安装目录；`--system` 使用系统目录（Windows `System32` / Linux `/usr/local/bin`）

* 托管目录默认：Windows `%USERPROFILE%\.sysenv\bin`，Ubuntu `~/.local/bin`；目录不在 PATH 时自动持久化加入（`--temporary` 则只打印片段）

### 示例



```
sys link mytool.exe                # 放入托管 bin 目录并自动加入 PATH
sys link ./script.py               # Windows 自动生成 script.cmd 垫片
sys link tool --name t             # 改名
sys link tool --method copy        # 强制用拷贝（跨盘时自动回退）
sys link tool --system             # 放到系统目录
sys link tool -f                   # 覆盖已存在的链接
```

## 4. 环境变量（`env`）

### 特性



* `get NAME`：读取当前进程生效值（含继承链）

* `set NAME VALUE`：默认**持久化**并同时更新当前进程；`--temporary` 仅打印 `export` / `$env:` 片段；`--scope machine` 写机器级

* `unset NAME`：删除持久化定义，并同步从当前进程移除

* `list`：列出全部已持久化的变量（键值对），支持 `--format` 输出为 env 文件 / JSON / 纯文本（与导入格式互通）

* 变量值包含空格、特殊字符时自动做 shell 转义，粘贴片段即可直接使用

### 示例



```
sys env get FOO
sys env set FOO bar                # 持久化（默认）
sys env set FOO bar --temporary    # 仅当前 shell（打印 export / $env: 片段）
sys env set FOO bar --scope machine
sys env unset FOO
sys env list
```

## 5. AI 模型 / Provider 查询（`ai`）

### 命令速查

| 子命令 | 用途 | 关键参数 / 示例 |
| ---- | ---- | ---- |
| `ai model` | 查 models.dev 模型目录（名称精确 / 子串 / 全量列表） | `--search` `--list` `--limit` `--date` `--open` `--model-type` `--provider` `--price` `--json` `-o` `--src openrouter`；`sys ai model gpt-4.1` |
| `ai cn-model` | 查 DataLearner 中文模型目录（1039 个，独立数据源） | `--search` `--list` `--limit` `--date` `--model-type` `--json` `-o`；`sys ai cn-model gpt-6-1-sol` |
| `ai provider` | 查 models.dev Provider 目录 | `--search` `--list` `--limit` `--json` `-o`；`sys ai provider openai` |
| `ai info` | 查本地配置（provider / model / 官网定价 / models 接口） | `info provider [KW]` `info model [KW]` `info sale-price PROV` `info list-model PROV` `info models PROV`；`-o json\|csv\|yaml\|md`；`--free[=ROUTER]`（providers / models） |
| `ai balance` | 用 api_key 查官方余额 | `sys ai balance minimax`（minimax / agnes / alibaba-cn / deepseek / tencent 有处理器，其余给控制台） |
| `ai price` | 查模型价格（models.dev 默认 / openrouter 目录） | `sys ai price openai,anthropic`；`--src openrouter` `--limit` `--json` `-o` |
| `ai chat` | 与配置的 LLM 聊天（OpenAI / Anthropic 兼容） | `-m` `--list-provider` `--list-model` `--no-stream` `--debug` `--update` `--test` `--query-model` `--free`；`sys ai chat "你好"` |
| `ai server` | 以 OpenAI 兼容 API 提供本地服务 | `sys ai server [ADDR]`（默认 127.0.0.1:10000）；`--help` 查运行状态 |
| `ai image` | 通过 Images API 生成图片 | `-o DIR` `-n N` `-s SIZE` `--url` `--list-provider` `--list-model`；`sys ai image "a red fox in the snow"` |
| `ai config` | 导出 / 应用 / 反向导入配置到其他 AI 工具 | 8 种格式；`-m` `-f` `-c` `--show` `--update` `--import` `--src` `--list`；`sys ai config codex --show` |

> 公共选项：`--refresh` 属 `ai` 层，可写在 `ai` 后任意位置（如 `sys ai --refresh model gpt-4.1` 或 `sys ai model gpt-4.1 --refresh`），强制重抓 24h 缓存数据。

### 特性



* 数据源为 [models.dev](https://models.dev) 官方公开数据 `https://models.dev/api.json`（约 5.3 MB，**226 个 Provider、8000+ 模型**）；`/models/` 与 `/providers/` 页面即由此数据渲染

* **24 小时本地缓存**：Windows `%LOCALAPPDATA%\sys\`、Linux `$XDG_CACHE_HOME` 或 `~/.cache/sys/`；`--refresh` 强制重新抓取

* 查询匹配：大小写不敏感，精确匹配模型 `id` / `canonical_model_id` / `name`，未命中自动降级为子串

* **canonical 优选**：多个 Provider 暴露同名模型时，按 `canonical_model_id` 前缀多数投票选出主条目（如 `gpt-4.1` → OpenAI），打印详情并提示其余 Provider 数量；也可用完整 id（`openai/gpt-4.1-mini`）精确定位

* `-s/--search`：子串搜索列表输出，`--limit` 控制条数（默认 20）

* `--list`：分页浏览全部模型 / Provider

* **无参默认全量**：`sys ai model` 不加参数直接列出全部模型（`--limit N` 仍可限制条数）

* **文本列表带列名**：无参 / `--list` / `--search` 的默认输出为对齐表格，表头 `ID  NAME  FAMILY  CONTEXT  TYPE  LAST UPDATED`（id / 名称 / 所属家族 / 上下文长度 / 模态类型 / 最后更新时间）

* `--date YYYY-MM-DD`：只显示 `last_updated` **晚于**该日期（严格大于）的模型；可与无参 / `--list` / `--search` / 名称查询组合（先按日期过滤再匹配），日期格式非法会报错

* `--open`：只显示 `open_weights: yes` 的模型；可与 `--date` 等过滤组合（先按日期、再按开放权重过滤）

* `--model-type TYPE[,TYPE...]`：只显示支持这些模态的模型，取值 `text / image / audio / video / pdf`（也接受中文 `文本 / 图像 / 语音 / 视频`）；**多个值用半角 `,` 或全角 `，` 分隔，取交集（模型须同时包含全部指定模态）**，如 `--model-type text,image` 只显示既能文本又能图像的模型；可与 `--date` / `--open` 等组合

* `-o/--output-format json|csv|yaml|md`：机器可读输出 ——`json` 输出 **JSON 数组**（与 `--json` 等价，单命中也是数组）；`csv` 输出带表头的 CSV 表格（model 24 列、provider 6 列，RFC-4180 转义）；`yaml` 输出 YAML 列表；`md` 输出 **Markdown**（`sys ai info` 系列输出无序列表 ——`- provider` 下挂 `api_base / api_key / console / docs` 子项；`ai model` / `ai provider` / `ai cn-model` 输出 Markdown 表格，单元格自动转义 `|` 与换行），适用于详情、搜索、列表与多匹配全部场景

* 模型详情字段：`id / name / provider / family / description / modalities / context / output limit / cost（每 1M tokens 的 input/output/cache_read 折算美元）/ reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` 等

### 示例



```
sys ai model                          # 无参：列出全部模型（sai model 同）
sys ai model --date 2026-10-01        # 只显示 last updated 晚于 2026-10-01 的模型
sys ai model --date 2026-10-01 --limit 10   # 上一条 + 只显示前 10 条
sys ai model --open --limit 10        # 只显示 open_weights: yes 的模型
sys ai model --open --date 2025-01-01 # 两个过滤组合：2025 年后更新且开放权重
sys ai model --model-type audio --limit 10   # 只显示支持语音/音频的模型（文本/图像/语音/视频同理）
sys ai model --model-type text,image --limit 10  # 多值取交集：同时支持文本和图像的模型
sys ai model --model-type "文本，图像" --limit 10 # 中文别名 + 全角逗号同样可用
sys ai model gpt-4.1                  # 精确查询；多 provider 同名时自动选 canonical 并提示其余
sys ai model openai/gpt-4.1-mini      # 用完整 id 精确定位
sys ai model -s qwen --limit 10       # 子串搜索（grep 风格列表）
sys ai model --list --limit 5         # 列出模型（共 8000+）
sys ai model gpt-4.1 --json           # 输出 JSON 数组（同 -o json）
sys ai model gpt-4.1 -o csv           # 输出 CSV 表格（24 列）
sys ai model -s qwen -o csv           # 搜索结果的 CSV 输出
sys ai provider openai -o csv         # Provider CSV（6 列）
sys ai info provider -o md            # Provider 信息输出 Markdown 表格
sys ai info model -o md               # 模型清单输出 Markdown 表格
sys ai model gpt-4.1 --refresh        # 强制重抓数据（--refresh 属 ai 层，也可写 ai --refresh model gpt-4.1）
sys ai provider openai                # Provider 详情（api / npm / models 数量与清单）
sys ai provider -s groq               # Provider 子串搜索
sys ai provider --list                # 列出全部 Provider
sys ai model --src openrouter --limit 10   # 数据源 openrouter.ai：id/name/context_length/pricing.prompt/created 列表
sys ai model --src openrouter -o json      # openrouter 数据源 JSON 输出
```

CSV 列：model 为 `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`；provider 为 `id,name,api,env,npm,models_count`。

#### 5.1 国内 AI 模型查询（`ai cn-model`）

数据源为 [datalearner](https://www.datalearner.com/ai-models/pretrained-models) 的预训练模型列表（服务端渲染 HTML，约 **1039 个模型**，分页抓取 + slug 去重），与 `ai model`（models.dev）相互独立。

* 查询与列表：支持按**名称精确查询**与 `-s/--search` 子串搜索（别名也会匹配），`--list` 列出全量目录；**没有** `--open`（该数据源不提供开放权重信息）

* `--date YYYY-MM-DD`：以卡片上的 **published 发布日期**为筛选标准，只显示**晚于**该日期的模型；单独使用 `--date`（不带名称 / 搜索词）时直接列出全部晚于该日期的模型

* `--model-type TYPE[,TYPE...]`：按**分类**筛选模型，取值 `text / image / audio / video / multimodal`（也接受中文 `文本 / 图像 / 语音 / 视频 / 多模态`）；**多个值用半角 `,` 或全角 `，` 分隔，取交集**（分类须同时匹配每个类型的关键词，如 `audio` 匹配“语音”分类、`image` 匹配“视觉 / 多模态”类）

* **精确命中时抓详情页**：单模型精确查询会额外抓取该模型的详情页，输出中附带 `context`（上下文长度，如 `1.05M`）、`modality`（输入/输出模态，如 `文本、图像 → 文本`）与 `parameters`（**模型规模 / 参数量**，如 `2350亿 / 220亿`），失败时静默降级（字段留空）；站点以「暂无数据」占位表示未知的字段同样留空

* 字段：`id（slug）/ name / provider（发布机构）/ aliases（又名）/ type（精选 / 预览版 / 开源模型 / 闭源模型等徽章）/ category（分类，如 推理大模型）/ context（上下文长度，详情页）/ modality（输入/输出模态，详情页）/ parameters（参数规模，详情页）/ published / url`

* 24 小时本地缓存（与 `ai model` 同一缓存目录，文件名 `datalearner-models.json`）；`--refresh` 强制重新抓取

* `--limit N` 限制列表条数（默认 20）；`--json` / `-o json` 输出 JSON 数组；`-o csv` 输出 11 列 CSV（`id,name,provider,aliases,type,category,context,modality,parameters,published,url`）

```
sys ai cn-model gpt-6-1-sol            # 精确查询（大小写不敏感；附带详情页 context / modality / parameters）
sys ai cn-model -s ernie --limit 10    # 子串搜索（别名也匹配）
sys ai cn-model --list --limit 20      # 列出全量目录（前 20 条）
sys ai cn-model --date 2026-09-28      # 列出所有 published 晚于该日期的模型
sys ai cn-model --model-type 语音       # 列出所有语音大模型
sys ai cn-model --model-type 语音,多模态 # 多值取交集：同时命中语音与多模态分类
sys ai cn-model -s qwen --date 2026-01-01   # 搜索 + 日期过滤
sys ai cn-model gpt-6-1-sol --json     # JSON 数组输出
sys ai cn-model -s ernie -o csv        # CSV 输出
sys ai cn-model gpt-6-1-sol --refresh  # 强制重新抓取（--refresh 属 ai 层）
```

#### 5.2 AI 聊天（`ai chat`）

按 OpenAI `/v1/chat/completions` 或 Anthropic Messages API 标准与配置好的 Provider 聊天。

* 配置文件默认位置 **`~/.sysenv/config.yaml`**（找不到会明确提示），可用 `-c/--config FILE` 覆盖；格式见 `doc/config.yaml`，脱敏模板见 `doc/config.example.yaml`

* `clients` 列表存放 Provider：必填 `name / api_base / api_key / models`（`models` 每项 `name` + 可选 `weight`，缺省权重 1，**取值范围 0-9**，可选 `max_tokens`、`max_input_tokens`、`type`、`group`）；`type` 为 `openai`（默认，兼容 `open`）或 `anthropic`，分别按 OpenAI / Claude API 标准发请求
  - `group`：模型的**分组**，可多分组、半角或全角逗号分隔（如 `group: fast, cheap`）；顶层 `model` 与 `-m` 可用 `group:<名>` 按组名选中**所有归属该组的模型**（跨 Provider、带权重轮询）

* **能力字段只读本地配置，除非显式 `--update`**：`max_input_tokens` / `type` 默认完全来自配置文件，缺失时**不查询 models.dev、不写回任何内容**——仅影响消息截断（见下）与请求中的可选字段；chat 不依赖外部数据源
  - `--update`：发送前把本次目标模型**缺失**的 `max_input_tokens` / `type` 从 models.dev（24h 缓存）补齐并**写回配置文件**（best effort：查询或写入失败仅警告、不阻塞请求；stderr 打印实际补齐内容）；字段已齐的模型不再查询。不传该参数则保持上面的零查询/零写回行为

* **消息长度截断**：发送前检查消息字符数是否超过该模型配置的 `max_input_tokens`，超过则截取到限制内（stderr 打印截断提示），防止超出上下文窗口；未配置 `max_input_tokens` 时不截断

* 顶层 `model` 选择模型，规则：
  - 缺失 → 第一个 Provider 的第 1 个模型
  - `provider:model` → Provider 名称与模型名称都匹配的那个模型
  - `provider:*` → 该 Provider 下**所有**模型，按 `weight` **带权重轮询**（缺省 1）
  - `model`（裸名）→ 所有 Provider 中名称匹配的模型合集，同样带权重轮询
  - `group:<名>` → 所有 Provider 中 `group` 列表含该名的模型合集（大小写不敏感），同样带权重轮询；未命中报错并列出可用分组
  - **多个选择**：顶层 `model` 与 `-m/--model` 都可用半角逗号 `,` 或全角逗号 `，` 分隔多个选择（如 `agnes:*,group:cheap`），按权重轮询使用

* `-m/--model MODEL`：**临时覆盖**顶层 `model`，规则与配置完全一致（`provider:model` / `provider:*` / 裸名 / `group:<名>` / 逗号分隔多选）

* `--list-provider`：列出配置中所有 Provider（名称 / type / 模型数）后退出；`--list-model`：按 Provider 分组列出所有模型（含权重与分组）后退出；两者可同时使用，均无需消息

* **动态权重轮询**：请求失败时若该模型 `weight > 1` 则 `-1` 并**写回配置文件**，随后自动尝试下一个模型；替补模型成功且 `weight < 9` 则 `+1` 并写回。权重越高的模型被选中概率越大，失败惩罚 / 成功奖励持续生效（weight 0 表示基本不参与轮询）

* 轮询状态持久化在配置同目录的 `chat_state.json`，多次调用会持续轮转

* 消息来源：命令行参数（多段自动拼接）；无参数时若 stdin 非终端则读取管道内容

* 顶层 `stream: true` 时默认 SSE 流式输出（逐字打印）；`--no-stream` 关闭；`--debug` 时自动改为非流式

* `--debug`：把**实际 HTTP 请求**（方法 / URL / 请求头 / 请求体）与**响应**（状态 / 响应头 / 响应体）打印到 stderr，不污染 stdout

* `--free[=ROUTER]`：**忽略配置文件，只用内置免费路由**（无配置文件时自动启用）：`kilo-auto/free`（Kilo Gateway，匿名直连）与 `blockrun/free`（BlockRun 匿名免费端点 `https://blockrun.ai/api/v1`，模型 `nvidia/llama-3.2-11b-vision`）始终可用；`openrouter/free` 仅在设置 `OPENROUTER_API_KEY` 后追加。`--free` 不带参数在所有可用路由间加权轮询；`--free=kilo` / `--free=blockrun` / `--free=openrouter` 固定单路由（别名接受 `kilo-auto` / `kilo-auto/free` / `blockrun/free` / `openrouter/free`），指定路由不可用时明确报错

```
sys ai chat "你好"                     # 用默认配置聊天（~/.sysenv/config.yaml）
sys ai chat 你好 世界                  # 多参数自动拼接
echo "帮我总结这段文字" | sys ai chat   # stdin 管道
sys ai chat "你好" -c doc/config.yaml  # 指定配置文件
sys ai chat "你好" --no-stream         # 关闭流式
sys ai chat "你好" --debug             # 打印实际请求与响应（含 header）
sys ai chat "hi" --free                # 用内置免费路由聊天（kilo-auto / blockrun，有 key 时含 openrouter）
sys ai chat "hi" --free=blockrun       # 固定 BlockRun 匿名免费端点（https://blockrun.ai/api/v1）
sys ai chat "你好" -m agnes:agnes-3.0-flash   # 覆盖顶层 model，只用 agnes 的该模型
sys ai chat "你好" -m "agnes:*,claude:claude-3-5-sonnet"  # 多模型逗号分隔，加权轮询
sys ai chat "你好" -m "group:cheap"               # 用分组名选中所有归属 cheap 组的模型
sys ai chat --list-provider            # 列出配置中的 Provider
sys ai chat --list-model               # 列出配置中的所有模型（含权重）
sys ai server                          # 以 OpenAI 兼容 API 对外提供聊天服务（默认 127.0.0.1:10000）
sys ai server 8080                     # 只给端口，host 用默认 127.0.0.1
sys ai server 0.0.0.0                  # 只给 IP，端口用默认 10000（对外网开放）
sys ai server 0.0.0.0:9000             # 同时指定 IP 与端口
sys ai server --help                   # 查询服务是否已启动：地址/端口 + 配置的 provider/model
```

* `sys ai server [ADDR]`：**把配置中的 Provider 以 OpenAI 兼容格式对外提供**（本地 HTTP 服务，默认 `127.0.0.1:10000`；`-c/--config` 指定配置文件）。提供 `POST /v1/chat/completions`（非流式与 SSE 流式透传到上游 Provider）与 `GET /v1/models`（列出全部已配置模型，ID 为 `{provider}:{model}`）。请求中的 `model` 使用与聊天相同的选择规则（`provider:model` / `provider:*` / 裸模型名 / `auto` = 配置默认）。仅支持 `type: openai` 的 Provider，anthropic 目标返回明确错误。地址可只给端口（如 `8080`，host 用默认）或只给 IP（如 `0.0.0.0`，端口用默认）。
* `sys ai server --help`：**查询服务状态**——先探测该地址（GET /v1/models，2 秒超时）是否已有服务在运行：已启动则打印当前地址/端口，未启动则提示 `server is NOT running`；两种情况都会继续打印配置信息（按 provider 分组：每个 provider 一行 name/api_base，其后缩进列出该 provider 的全部模型 `{provider}:{name}`）。

#### 5.3 AI 图片生成（`ai image`）

通过 Provider 的 OpenAI 兼容 **Images API**（`POST {api_base}/images/generations`，Bearer 鉴权）生成图片并保存到本地。

* 配置文件与 `ai chat` 完全一致（默认 `~/.sysenv/config.yaml`，`-c/--config` 覆盖），只支持 **OpenAI 兼容 Provider（`type: openai`）**；`type: anthropic` 目标在发请求前明确报错（Anthropic 没有 Images API）

* **模型选择与 `ai chat` 同一套规则**：顶层 `model` 或 `-m/--model`（`{provider}:{model}` / `{model}` / `provider:*` / `group:<名>` / 逗号分隔多选），多目标按 `weight` 加权轮询，失败自动降权并尝试下一个模型，替补成功升权，权重写回配置文件

* **参数**：`-o/--output DIR` 保存目录（默认当前目录，自动创建）；`-n/--count N` 生成数量（默认 1）；`-s/--size SIZE` 尺寸（默认 `1024x1024`，原样传给 API，如 `512x512` / `1792x1024`）；`--url` 请求图片 URL 并下载保存（默认请求 `b64_json`，本地解码保存，无需二次网络请求）

* **保存**：文件名 `sys-ai-image-<YYYYMMDD-HHMMSS>-<序号>.<扩展名>`，扩展名按文件头魔数识别（png / jpg / gif / webp），重名自动追加 `-1`、`-2` 后缀不覆盖；每张图片的保存路径打印到 stdout

* 提示词来源：命令行参数（多段自动拼接）；无参数时若 stdin 非终端则读取管道内容；`--debug` 打印实际 HTTP 请求与响应到 stderr

```
sys ai image "a red fox in the snow"                  # 默认配置 + 默认模型，保存到当前目录
sys ai image 一只 雪地里的 红色狐狸                   # 多参数自动拼接（空格分隔）
echo "赛博朋克风格的城市夜景" | sys ai image          # stdin 管道
sys ai image "樱花树下的小猫" -o pics -n 2 -s 512x512 # 2 张 512x512，保存到 pics/
sys ai image "海报主视觉" -m bigmodel:cogview-4       # 覆盖顶层 model，只用 bigmodel 的该模型
sys ai image "风景" -m "agnes:*,bigmodel:cogview-4"   # 多模型逗号分隔，加权轮询
sys ai image "风景" --url -o pics                     # 请求 URL 版并下载保存
sys ai image --list-provider --list-model             # 列出配置中的 Provider / 模型
```

#### 5.4 本地配置查询（`ai info`）

检查本地配置文件（默认 `~/.sysenv/config.yaml`，`-c/--config` 覆盖）与 models.dev 模型价格。**不带 FIELD 参数时默认同时打印 provider 表（name/api_base/api_key/DOCS/CONSOLE）与全部模型清单（`{provider}:{name}`）**。

* `info provider [KEYWORD]`：列出 `clients` 中全部 provider 的 `name / api_base / api_key` 及官方 `DOCS`（帮助文档）与 `CONSOLE`（控制台）地址（按 provider 名称匹配内置收录表，未收录显示 `-`）；带 KEYWORD 时只保留名称包含该词的 provider（大小写不敏感），无匹配报错并列出可用名称；**默认输出 YAML 列表**（`-o yaml` 等价），`-o json` / `--json` 输出 JSON 数组，`-o csv` 输出 CSV 表（JSON/YAML 字段 `name` / `api_base` / `api_key` / `docs` / `console`）
* `info model [KEYWORD]`：列出全部 provider 下的 model，格式 `{provider}:{name}`；带 KEYWORD 时按 provider 名称包含匹配（支持 `provider:model` / `provider:*` 精确选择），无 provider 命中时回退按 model 名称匹配；`-o json` / `-o csv` / `--json` 输出 JSON 数组或 CSV 表（JSON 字段 `provider` / `name`）
* `info providers` / `info models` 支持 **`--free[=ROUTER]`**：跳过本地配置文件，改用内置免费路由——`kilo-auto/free`（匿名）与 `blockrun/free`（匿名）始终可用，`openrouter/free` 仅当设置 `OPENROUTER_API_KEY` 后追加；`--free` 裸用列出全部可用路由，`--free=kilo` / `--free=blockrun` / `--free=openrouter` 固定单路由（与 `ai chat --free` 同一套路由）
* `info sale-price PROVIDER`：抓取官网定价页并列出全部模型销售价。agnes 抓取 `wiki.agnes-ai.cn` 定价页（文本/图片/视频模型，刊例价 + 现价，人民币）；minimax 抓取 `platform.minimaxi.com` 定价页（语言模型输入/输出/缓存价格，元/百万 tokens）；alibaba-cn 给出百炼官方模型列表与计费说明页地址；其余 provider 给出官网定价页地址。定价页 24h 缓存，`--refresh` 强制重抓
* `info list-model PROVIDER` / `info models PROVIDER`：用该 provider 自己的 models 接口实时查询其可用模型列表（与 `sys ai chat --query-model PROVIDER` 同一功能）——OpenAI 兼容 provider 请求 `GET {api_base}/models`（`models` 直接追加在配置的 api_base 后，如百度千帆配置 `https://qianfan.baidubce.com/v2` 时请求 `/v2/models`，不是 `/v2/v1/models`；Bearer api_key），anthropic 类型请求 `GET {api_base}/v1/models`（api_base 已以 `/v1` 结尾时直接用 `{api_base}/models`，x-api-key / anthropic-version）；Anthropic `next_page` 分页自动翻页（上限 20 页）。文本视图打印 `MODEL / OWNED_BY / CREATED` 对齐表（openai 的 `owned_by` / `created`、anthropic 的 `display_name` / `created_at` 自动归一，`created` 时间戳在文本视图自动转为本地时间 `YYYY-MM-DD HH:MM:SS`）；`-o json` / `-o csv` / `--json` 输出 JSON 数组或 CSV 表（字段 `id` / `owned_by` / `created`，保持 API 原始时间戳）。实时查询，不走缓存
* **余额查询**（v0.6.17 起 `info balance` 已提升为独立子命令）：用 provider 的 `api_key` 查官方余额请用 **`sys ai balance PROVIDER`**（见 5.6）；**模型价格查询**（models.dev / openrouter 目录）用 **`sys ai price`**（见 5.5）

```
sys ai info                                # 无参：provider 表 + 全部模型清单
sys ai info provider                     # 默认 YAML 输出：全部 provider 信息
sys ai info provider -o yaml             # 等价于默认（YAML 列表）
sys ai info provider agnes               # 只显示名称包含 agnes 的 provider（YAML）
sys ai info provider -o json             # 全部 provider 的 JSON 数组输出
sys ai info provider agnes -o csv        # 过滤 + CSV 输出
sys ai info provider --json              # 等价于 -o json
sys ai info model                        # 列出全部模型，格式 provider:model
sys ai info model modelscope             # 只显示 modelscope 下的模型
sys ai info model deepseek               # 无 provider 命中时按模型名匹配
sys ai info model agnes:3.0              # 精确选择 agnes 下名称含 3.0 的模型
sys ai info model modelscope:*           # modelscope 全部模型
sys ai price openai,anthropic            # 查询两个 provider 的模型价格列表（models.dev）
sys ai price "openai，deepseek"          # 全角逗号同样支持
sys ai price --src openrouter            # openrouter.ai 全量模型的 prompt/completion 价格
sys ai price deepseek --src openrouter   # openrouter 数据源 + 模型名称过滤
sys ai price --src openrouter --limit 10 # 只显示前 10 条
sys ai info providers --free             # 用内置免费路由列出 provider（不读配置文件）
sys ai info models --free=blockrun       # 固定 blockrun 免费路由列出模型
sys ai info sale-price agnes             # 抓取 agnes 官网定价页，列出全部模型销售价
sys ai info list-model agnes             # 用 agnes 的 models 接口查询可用模型（实时）
sys ai info list-model anthropic -o json # anthropic 类型：/v1/models，JSON 数组输出
sys ai info models baidu                 # 与 --query-model 同功能：查 baidu 的 /v2/models（实时）
```

#### 5.5 模型价格查询（`ai price`）

模型价格数据来自 [models.dev](https://models.dev)（默认，与 `ai model` 同一数据源）或 openrouter.ai 官方定价目录（`--src openrouter`）。

* **`src=models`（默认）**：`sys ai price openai,anthropic` 把逗号分隔的 provider 名称在 models.dev 中解析，列出这些 provider 的**全部模型**及其价格（每 1M tokens 的 input / output / cache_read 折算美元，`--refresh` 强制重抓 24h 缓存）。半角 `,` 与全角 `，` 均可
* **`src=openrouter`**：`sys ai price --src openrouter` 拉取 openrouter.ai 全量模型目录的 prompt / completion 价格（每 1M tokens，24h 缓存，`--refresh` 重抓）；无需 provider 名称，可跟逗号分隔的模型 id / 名称关键词做过滤（`sys ai price deepseek --src openrouter`），`--limit N` 限制条数，`--json` / `-o json|csv|yaml|md` 切换输出格式
* 与 `ai info`（本地配置查询）互补：`info` 看配置里有什么、`price` 看各模型官方定价

```
sys ai price openai,anthropic            # models.dev：两个 provider 的全部模型价格
sys ai price "openai，deepseek"          # 全角逗号同样支持
sys ai price --src openrouter            # openrouter.ai 全量模型价格（每 1M tokens）
sys ai price deepseek --src openrouter   # openrouter 目录 + 模型名称过滤
sys ai price --src openrouter --limit 10 # 只显示前 10 条
sys ai price --src openrouter -o csv     # CSV 输出
```

#### 5.6 官方余额查询（`ai balance`）

用 provider 配置的 `api_key` 到官方接口实时查询余额（**始终实时，不走缓存**）。`-c/--config FILE` 指定配置文件，`PROVIDER` 为配置 `clients` 下的 provider 名称（大小写不敏感，支持子串唯一匹配）。

* **`minimax`**：官方 `GET /v1/token_plan/remains`——列出 Token Plan 下每个模型的剩余 / 总额 / 已用额度；无 Token Plan 订阅（按量付费）时给出平台余额查看入口
* **`agnes`**：OpenAI 兼容 billing 探测（credit_grants / subscription / usage，无余额字段时提示控制台）
* **`alibaba-cn`**（别名 alibaba / dashscope / bailian）：官方 `GET /api/v1/models/limits`——返回该 Key 下各模型的用量限额 / 限流配额（可验证 Key 有效性；官方无 Key 级现金余额接口，提示到百炼控制台）
* **`deepseek`**：官方 `user/balance` 真实查询
* **`tencent`**（别名 tencentmaas / tokenhub）：TokenHub 管控面需要 SecretId / SecretKey（与配置的 API Key 不同），给出控制台指引
* **`modelscope` / `sensenova` / `bigmodel` / `amd` / `anspire`**：官方未开放 Key 级余额接口，命令给出对应控制台地址
* 未收录的 provider 名明确报错

```
sys ai balance minimax              # MiniMax Token Plan 剩余额度（或按量付费入口）
sys ai balance deepseek             # DeepSeek user/balance 真实查询
sys ai balance alibaba-cn           # 阿里云百炼：官方 limits 限额 + 控制台指引
sys ai balance tencent              # TokenHub 控制台指引（SecretId/SecretKey）
sys ai balance agnes -c 测试\config.yaml   # 指定配置文件
```

#### 5.7 配置导出 / 应用（`ai config`）

把本地配置（`~/.sysenv/config.yaml` 的 `clients` 段）导出为其他 AI 工具的配置格式，或直接应用到该软件的配置文件：

* `ai config FORMAT [-m MODEL] [-f FILE] [-c FILE] [--show] [--update]`，`FORMAT` 支持：
  * **`codex`**（TOML）：每个 Provider 一个 `[model_providers.<name>]` 段（`name` / `base_url` / `env_key` / `wire_api = "chat"`）；codex 只从环境变量读 key，导出时生成 `SYS_<PROVIDER>_API_KEY` 命名并打印 `export` 提示到 stderr；顶层 `model` 选中第一个模型的 `provider.model`
  * **`opencode`**（JSON）：每个 Provider 一个条目（`npm: "@ai-sdk/openai-compatible"` + `options.baseURL` / `options.apiKey` 内联 + `models` 映射），可直接合并进 `opencode.json`
  * **`litellm`**（YAML）：每个模型一条 `model_list` 条目，暴露名 `{provider}:{model}`（防冲突），路由到 `openai/{model}`，`api_base` / `api_key` 内联
  * **`freellmapi`**（JSON）：`customProviders` 数组，每个 Provider 一个 `baseUrl` / `label` / `models` 条目（`supportsTools: true`），可合并进 `freellmapi.config.json`
  * **`codebuddy`**（models.json）：每个模型一条 `id` / `name` / `vendor` / `url`（`{api_base}/chat/completions`）/ 内联 `apiKey`；配置里有 `max_input_tokens` / `max_tokens` / `type`（含 `image`）时带上 `maxInputTokens` / `maxOutputTokens` / `supportsImages`；同名模型去重，`availableModels` 同步；默认路径 `~/.codebuddy/models.json`
  * **`workbuddy`**（models.json，**顶层纯数组**）：条目字段同 codebuddy（`id` / `name` / `vendor` / `url` / `apiKey` / 可选 `maxInputTokens` / `maxOutputTokens` / `supportsImages`）；workbuddy 真实 schema 是顶层数组（非 `{"models":...}` 包装），导出为数组、`--update` 合并后仍保持数组；默认路径 `~/.workbuddy/models.json`
  * **`trae`**（trae_cli.yaml）：`models` 列表，OpenAI 兼容 Provider 用 `open_ai` 块、anthropic 用 `claude` 块，按模型 id 去重；默认路径 Windows `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`（Linux `$XDG_CONFIG_HOME` 或 `~/.config/trae_cli/trae_cli.yaml`）
  * **`zcoder`**（ZCode `config.json`）：每个 Provider 一个 `provider.<id>` 块（id = Provider 名小写化、非法字符转 `-`），含 `kind`（anthropic / openai-compatible）、`options.baseURL` / `options.apiKey`、`models`（按模型 id 键控的对象）；`model.main` / `model.lite` 指向所选模型（`provider/model`）；`--update` 保留已有 provider 块与其它顶层键、`main/lite` 仅缺省时填入；默认路径 `~/.zcode/cli/config.json`（Windows `%USERPROFILE%\.zcode\cli\config.json`）
* `-m/--models MODEL` 选择导出范围（缺省 = **与 `sys ai chat` 相同的默认模型**：配置顶层 `model:` 选择器解析结果，未配置时取第一个 Provider 的第一个模型），与 `sys ai chat -m` 同格式：`provider:*`（指定 Provider 的全部模型）/ `provider:model`（单个模型）/ 裸模型名（跨 Provider 匹配），半角/全角逗号分隔可指定多个，重叠选择去重（无匹配明确报错）；旧式位置参数 `SELECT` 等价于单个 `-m` 值（二者互斥）
* `--show`：**只查看不写入**——先打印对应软件的配置文件地址（codex → `~/.codex/config.toml`、opencode → `~/.config/opencode/opencode.json`、litellm → `~/.litellm/config.yaml`、freellmapi → 当前目录 `freellmapi.config.json`、codebuddy → `~/.codebuddy/models.json`、workbuddy → `~/.workbuddy/models.json`、trae → `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`、zcoder → `~/.zcode/cli/config.json`；无默认路径时提示用 `-f` 指定），再打印将要写入该软件的配置信息
* `-f/--file FILE` **写入文件**（缺省打印到 stdout）：写入前先把已有文件备份为 `FILE.bak-YYYYMMDD-HHMMSS`（同目录），写入后打印实际修改信息（导出的模型数 / 目标文件路径 / 备份文件路径）；目标文件不存在时新建并注明；`-c/--config` 指定配置文件
* `--update` **就地合并更新**（仅 codebuddy / workbuddy / trae / zcoder）：按指定格式到该软件默认配置目录找文件（`-f` 可覆盖路径）；文件不存在则提示 `config file not found ... nothing to update`；存在则先备份 `.bak-*`，再合并写入缺失的模型条目（保留已有条目与其它顶层键），成功后提示 `added N model(s): ...`；与 `--show` 互斥
* `--import` **反向导入**（codebuddy / workbuddy / opencode / zcoder；与 `--show` / `--update` / `--list` 互斥）：从该软件配置文件读取 Provider / 模型参数，合并写回本地配置 `~/.sysenv/config.yaml`（`-c` 指定其它目标；`-f FILE` 指定源头文件，缺省用该格式的默认路径；源文件不存在则报错）。`codebuddy` / `workbuddy` 解析 `models.json`（数组根或 `{"models":[...]}` 根均可）：条目 `id` → 模型名、`url` 去掉尾部 `/chat/completions` 还原 `api_base`、`apiKey` → `api_key`、`maxInputTokens` / `maxOutputTokens` / `supportsImages` 映射到 `max_input_tokens` / `max_tokens` / `type`；`vendor` 携带真实名时作 Provider 名，否则（如 workbuddy 手加的 `Custom`）取 `api_base` 主机名；`opencode` 解析 `provider.<id>` 块的 `options.baseURL` / `options.apiKey` / `models`；`zcoder` 解析 `provider.<id>` 块的 `options.baseURL` / `options.apiKey` / `models` 与 `kind`（anthropic → `type: anthropic`）。合并策略：**新增优先，本地已有同 api_base 的 Provider 保留本地**——本地没有该端点的 Provider 追加完整块（备份后写入），已有 Provider 只追加本地缺失的模型条目，同名模型保留本地并提示 `skipped`；缺 `id` / `url` / `apiKey`（或 zcoder / opencode 缺 key / 无模型）的条目跳过并提示。**以 api_base 判定同一 Provider**：一个 Provider 的身份是其端点——来源中指向同一 api_base（尾部斜杠与大小写忽略）的条目一律合并进同一个块（首个条目的名字 / kind / api_key 保留，模型合并去重），config.yaml 不会为同一端点写入多个条目；指向不同 api_base 的条目即使名字相同也是不同 Provider，各自成块。加载配置时按 api_base 检测重复并打印 warning（不阻断）。全部已存在时提示 `nothing to import` 且不写文件
* **导出同样按 api_base 归并**：上述 8 种格式（codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder）导出时，选中模型中指向同一 api_base（尾部斜杠与大小写忽略）的 Provider 一律输出为一个条目/块（首个块的名字 / api_key 生效，模型跨块合并、按模型名去重），不会为同一端点建立多个条目；同名但端点不同的 Provider 保持为不同条目，条目名冲突时以 `名字@主机名` 唯一化（如 `openroute@openrouter.ai`）
* `--src DIR` **指定 agent 配置目录**：把该格式的约定配置文件名（codex `config.toml`、opencode `opencode.json`、litellm `config.yaml`、freellmapi `freellmapi.config.json`、codebuddy / workbuddy `models.json`、trae `trae_cli.yaml`、zcoder `config.json`）直接在 `DIR` 下查找，替代默认的 home 相对路径，适用于 agent 装在非默认位置时的 `--show` / `--update` / `--import` / `--list`（`-f FILE` 显式路径始终优先于 `--src`）
* `--list`：只列出配置文件**实际存在**的格式及其路径（默认位置或 `--src DIR` 下；不存在的格式过滤掉，全部不存在时提示 `no existing config files found`；此时可省略 `FORMAT`；与 `--show` / `--update` 互斥）

```
sys ai config codex --show               # 打印 codex 配置文件地址 + 将写入的配置（不写入）
sys ai config codex -f ~/.codex/config.toml   # 备份原配置 → 写入 → 打印实际修改信息
sys ai config codex                      # 全部模型导出为 codex TOML（stdout）
sys ai config opencode -f opencode.json  # 写入文件（自动备份原文件）
sys ai config litellm -m "agnes:*"       # 只导出 agnes 的模型（-m 与 chat 同格式）
sys ai config codebuddy -m "modelscope:*,minimax:MiniMax-M2.7"  # 多个选择
sys ai config trae --update              # 备份默认 trae_cli.yaml → 合并缺失模型 → 提示新增
sys ai config workbuddy --update -f ~/.workbuddy/models.json   # 指定文件更新
sys ai config zcoder -m "amd:*" --update   # 合并进 ZCode config.json（保留已有 provider 与其它键）
sys ai config workbuddy --import         # 从 ~/.workbuddy/models.json 反向导入到 ~/.sysenv/config.yaml（先备份）
sys ai config opencode --import --src D:\tools\opencode   # 从指定目录读 opencode.json 导入（agent 装在不默认位置）
sys ai config zcoder --import -f 某处\config.json   # 指定源头文件导入
sys ai config codebuddy --import -c 测试\config.yaml   # 导入到指定 sysenv 配置
sys ai config --list --src D:\tools\opencode        # 只列出该目录下存在的配置文件
sys ai config --list                    # 只列出本机默认路径下存在的配置文件
```

#### 5.8 文本搜索与替换（`file`）

fd/sd 风格：搜索 stdin 或目录树中的文本/源码文件，或就地替换字符串。匹配在 Unicode 字符层进行（`-i` 逐字符大小写折叠，中文等非 ASCII 同样正确）。

* 1 个参数 `file PATTERN`：从标准输入读取并输出匹配行（管道用法：`type a.txt | sys file hello`）；**带扩展名的参数（如 `me.txt`）改为直接显示该文件内容**（cat 风格），用引号包裹（`"me.txt"`）可强制按字符串搜索
* 2 个参数 `file PATTERN PATH`：在 PATH（文件或目录树）的所有已知文本与源码文件（txt/md/py/java/c/...）中搜索 PATTERN，输出 `路径:行号:内容`
* 3 个参数 `file OLD NEW PATH`：在 PATH 树中把 OLD 就地替换为 NEW，输出每个文件的替换数与汇总

选项：

* `-e EXT`：只搜索指定扩展名的文件（可多次，前导点可省略，如 `-e py -e md`）
* `-i`：忽略大小写（搜索与替换均生效）
* `-t`：只搜索纯文本文件（txt/md/log/csv/json/...），排除源码文件
* `-w`：整词匹配（前后字符非字母/数字/下划线）
* `-c NUM`：显示匹配行的前后 NUM 行（多组间以 `--` 分隔）
* `-n`：在输出行前加上行号（stdin 搜索；目录搜索输出本身已是 `路径:行号:内容`）
* `-l`：统计行数而非输出内容——`-l FILE` 统计文件总行数、无参数时统计 stdin 总行数；带 PATTERN 时统计**匹配行数**（stdin 输出 `N`；目录树输出每个文件的 `路径: N`，无匹配的文件不显示）
* `-md5 FILE [MD5]` / `-sha FILE [SHA1]`：**哈希模式**（优先级最高）。对 FILE 做 MD5 / SHA-1 运算并输出小写十六进制摘要；再给一个参数则与期望值比对——相等输出 `match: <摘要>`，不等输出 `mismatch: computed <摘要> != expected <期望>`；期望值大小写不敏感，但长度/字符不合法时报 `expected md5/sha1 must be N hex characters`。大文件流式哈希（64 KiB 分块，内存有界）；`-md5` 与 `-sha` 同时给出报错；单字符 `-m` 等其它参数不受影响（标记需完整匹配 `-md5`/`-sha`/`--md5`/`--sha`）
* `--enc SRC [DST]`：**编码转换模式**（优先级高于哈希）。把文本文件在 `utf-8` / `gbk` / `gb2312` 之间转换（**默认转 utf-8**；`--from ENC` 指定源编码，缺省自动检测；`--to ENC` 指定目标编码）；DST 给出则写入该文件，缺省输出到 stdout；**DST 必须不同于 SRC**（不允许原地覆盖）。`gb2312` 按 GBK 编解码（GB2312 是 GBK 子集）；自动检测与 `data` 同一套启发（UTF-8 严格 → GBK 清洗回读对照），检测不出时报错提示 `--from`；UTF-8 BOM 会被剥离、输出不带 BOM。别名 `--encode` / `--convert`，`-f ENC` 等价 `--from ENC`（注意用双杠 `--enc`，单杠 `-enc` 会被 `-e` 吃掉）

文件属性筛选（fd 风格，可与搜索/替换/列文件叠加）：

* `-S SIZE` / `--size SIZE`：只处理大小达到指定值的文件。纯数字按字节；`2k`/`2m`/`2g`/`2t`（或 kb/mb/gb/tb）按 1024 进制，支持小数（`1.5m`），大小写不敏感
* `--newer TIME`：只处理修改时间不早于 TIME 的文件；`--older TIME`：只处理早于 TIME 的文件。时间格式 `YYYY-MM-DD [HH:MM[:SS]]`（`T` 或斜杠分隔也可），按本地时区解释
* `-d NUM`：只递归 NUM 级子目录（`-d 0` 仅当前目录）

给出任一筛选且不带 PATTERN 时进入**列文件模式**（每行一个路径，默认当前目录，`-e`/`-t` 可附加筛选）；带 PATTERN 或 OLD NEW 时筛选叠加到字符串搜索与就地替换。

遍历规则：递归目录树时跳过隐藏项（`.` 开头）与常见噪音目录（`.git` / `node_modules` / `target` / `dist` / `build` / `__pycache__` 等）；二进制文件（含 NUL 字节）自动跳过；替换模式同样遵守 `-e` / `-t` / `-i` / `-w`。

```
sys file hello                              # stdin 管道搜索（type a.txt | sys file hello）
sys file -n hello                           # stdin 搜索，输出行前加行号
sys file me.txt                             # 显示 me.txt 文件内容
sys file "me.txt"                           # 引号包裹：仍按字符串搜索（stdin 中找 me.txt）
sys file hello D:\projects                  # 在 D:\projects 树中搜索
sys file hello D:\projects -e py -e md      # 只搜 .py 和 .md
sys file HELLO D:\projects -i               # 忽略大小写
sys file hello D:\projects -t               # 只搜纯文本（不含源码）
sys file hello D:\projects -w -c 2          # 整词 + 前后 2 行上下文
sys file -l me.txt                          # 统计 me.txt 的总行数
type a.txt | sys file -l                    # 统计 stdin 输入的总行数
type a.txt | sys file -l hello              # 统计 stdin 中匹配 hello 的行数
sys file -l hello D:\projects               # 统计每个文件的匹配行数（路径: N）
sys file -md5 me.txt                        # 打印 me.txt 的 MD5
sys file -sha me.txt a94a8fe5...            # 比对 SHA-1（match / mismatch）
sys file --enc old.log new.log --from gbk   # GBK 日志转 UTF-8 写入 new.log（默认目标 utf-8）
sys file --enc old.log                      # 自动检测编码，转 UTF-8 输出到 stdout
sys file --enc a.txt b.txt --to gbk         # UTF-8 转 GBK
sys file hello hi D:\projects               # 就地替换 hello -> hi
sys file HELLO hi D:\projects -i            # 忽略大小写替换
sys file -S 2m                              # 列出当前目录下 >= 2 MiB 的文件
sys file -S 5000 D:\data                    # 列出 D:\data 下 >= 5000 字节的文件
sys file -S 2m -e bin -d 1 D:\data          # >= 2MiB 的 .bin 文件，只递归 1 层
sys file --newer "2026-10-01" D:\data       # 修改时间 >= 2026-10-01 的文件
sys file --older "2026-10-01 12:00" D:\data # 修改时间早于该时刻的文件
sys file hello D:\data -S 1m                # 只在 >= 1 MiB 的文件中搜 hello
sys file -md5 me.txt                        # 输出 me.txt 的 MD5 摘要
sys file -md5 me.txt 900150983cd24fb0d6963f7d28e17f72   # 比对：相等输出 match，否则 mismatch
sys file -sha me.txt da39a3ee5e6b4b0d3255bfef95601890afd80709  # SHA-1 同理
```

## 6. 进程管理（`task`）

### 特性

* **无参默认全量**：`sys task`（`stask`）不加参数直接列出全部进程，等价于 `task list`

* `task list [NAME]`：列出全部进程的 **PID / 名称 / 可执行文件路径**；`NAME` 按名称模糊匹配（子串、大小写不敏感），传数字则按 PID 精确查询

* `-o/--port <PORT>`：只显示**占用该端口**的进程（如 `task -o 8080` 或 `task list -o 8080`），可与名称 / PID 过滤组合；Windows 用 `netstat -ano`，Linux 用 `ss -ltnp`
* `task kill <PID|名称>`：按 PID 或名称终止进程；名称模糊匹配会终止**全部**命中进程；`-f/--force` 强制终止（Linux 发送 SIGKILL，默认 SIGTERM）；无权限等失败项单独提示，不中断其余
* 跨平台实现：Windows 使用 Toolhelp 快照 + `TerminateProcess`，Linux 读取 `/proc` + `kill`

### 示例

```
sys task                       # 无参：列出全部进程（等价于 task list）
sys task -o 8080               # 查看占用 8080 端口的进程
sys task list -o 8080          # 同上（显式 list 写法）
sys task list chrome           # 按名称模糊匹配（子串，大小写不敏感）
sys task list 1234             # 按 PID 查询
sys task kill 1234             # 按 PID 终止
sys task kill notepad          # 按名称模糊匹配，终止全部命中的进程
sys task kill -f 1234          # 强制终止（Linux 发送 SIGKILL）
```

## 7. httpie 兼容 HTTP 客户端（`http`）

参数与 [httpie](https://httpie.io) 保持一致（子集）。

### 特性



* 方法自动推导：URL 项中不含方法时自动 GET；含数据项时自动 POST

* **默认 JSON**：`key=value` 自动构造成 JSON 对象；**所有请求默认带 `Content-Type: application/json`**（即使无数据项；`--raw` / `@file` / `--file` / stdin 原始体亦同；可用 `-f/--form`、`--multipart` 或显式 `Content-Type:xxx` 头覆盖）；`-f` 切换表单、`--multipart` 文件上传

* 完整请求项语法（与 httpie 一致）：数据字段、原始 JSON、查询参数、请求头、multipart 文件、`@file` 原始体、嵌套 JSON 构建（`a[b][c]=v`、`a[]=v`、`a[1]=v`）

* 输出控制：`-p/--print`（`BHbh` 任意组合）、`-h` 仅头、`-b` 仅体、`-m` 状态行、`-v` 全量；终端下默认 `hb`，管道 / 重定向下自动仅 `b`

* `-o/--output` 响应体存文件（其余信息打 stderr）、`-d/--download` wget 式下载

* 认证：`-a user:pass` Basic、`-A bearer -a TOKEN` Bearer

* 网络行为：`-F/--follow` 跟随重定向、`--max-redirects`、`--timeout`、`--proxy`、`--verify no` 跳过证书校验、`--offline` 只构建并打印请求不发送、`--curl` 构建并打印等价的 curl 命令（不发送，隐含 `--offline`）、`-I/--ignore-stdin`

* `--check-status`：3xx → 退出码 3，4xx → 4，5xx → 5（脚本友好）

* `--help`：打印接口参数说明与示例（http 子命令的 `-h` 是 httpie 语义的“只打印响应头”）

* `--debug`：把**实际 HTTP 请求**（方法 / URL / 请求头 / 请求体，含 Content-Length）与**响应**（状态行 / 响应头 / 响应体）打印到 stderr，stdout 保持正常输出，便于排查真实发包内容

* `--curl`：构建并打印与请求**完全等价**的 curl 命令（方法 / URL / 查询参数 / 请求头 / 请求体 / 认证 / 代理 / 重定向 / 超时 / 证书 / 输出），不发送请求；JSON、表单、multipart（`-F` 文本字段与文件上传）均按实际发送内容转换；`--file FILE` 保留为 `--data-binary @FILE`，其余原始体内联；命令按 POSIX shell 单引号引用（bash / zsh / WSL / git-bash / PowerShell 均可直接运行）

* stdin 管道直接作为原始请求体；`--raw` 显式指定原始体

* 终端美化：默认对 JSON 响应做缩进格式化，`--pretty none` 关闭

### 示例



```
sys http pie.dev/get                       # GET（终端默认显示状态行+头+体）
sys http pie.dev/post name=John age:=29    # 无方法时自动 POST；默认 JSON
sys http -f POST pie.dev/post name='John Smith'   # 表单
sys http -v pie.dev/get                    # 打印完整请求与响应
sys http -h pie.dev/get                    # 只打印响应头
sys http GET pie.dev/get q==httpie per_page==1   # 查询参数
sys http pie.dev/post X-API-Token:123 name=John  # 请求头 + JSON 字段
sys http -d pie.dev/image.png              # wget 式下载
sys http -o out.json pie.dev/get           # 响应体存文件（其余打到 stderr）
sys http POST pie.dev/post @data.json      # 文件作为原始请求体
sys http POST pie.dev/post --file data.json # --file 读取本地文件作为原始请求体（等价 @FILE）
sys http POST pie.dev/post note=@note.txt   # 字段值以 @ 开头则读取本地文件（特殊字符自动转义）
sys http pie.dev/post cv@resume.pdf        # multipart 文件上传
sys http -a user:pass pie.dev/anything     # Basic Auth
sys http -A bearer -a TOKEN pie.dev/anything   # Bearer Token
sys http -F --max-redirects 5 pie.dev/     # 跟随重定向
sys http --check-status pie.dev/404        # 退出码 = 4
sys http --offline pie.dev/post a=1        # 只构建并打印请求，不发送
sys http --curl pie.dev/post name=John age:=29   # 打印等价的 curl 命令（不发送）
sys http --help                            # 打印接口参数说明与示例
sys http --debug pie.dev/post a=1 b:=2     # 打印实际请求与响应（含 header）
sys http POST pie.dev/post --raw '{"a":1}' # 显式原始体
sys http pie.dev/post -- -name=foo         # 以 - 开头的字段名需跟在 -- 之后
echo '{"a":1}' | sys http POST pie.dev/post  # stdin 作为原始体
sys http --verify no https://self-signed.example  # 跳过证书校验
```

> PowerShell 注意：PS 5.1 会剥掉传给原生程序参数中的内嵌双引号，含引号的 JSON / 原始体请用
> `\"`
> 转义、单引号包裹或在 cmd/bash 下执行。

### 请求项语法



| 项                                                       | 含义                                   |
| ------------------------------------------------------- | ------------------------------------ |
| `key=value`                                             | 数据字段（默认 JSON，`-f` 时为表单）              |
| `key:=json`                                             | 原始 JSON 值（数字 / 布尔 / 对象 / 数组）         |
| `key==value`                                            | URL 查询参数                             |
| `key:value`                                             | 请求头（`key:` 空值 = 取消默认头，`key;` = 发送空头） |
| `key@file`                                              | multipart 文件上传（`;type=mime` 可指定类型）   |
| `key=@file` / `key:=@file` / `key==@file` / `key:@file` | 从文件读取字段 / JSON / 查询 / 头的值（内容中的引号、反斜杠等特殊字符在 JSON 序列化时自动转义；自动剥离 UTF-8 BOM 与尾部换行）            |
| `@file`                                                 | 原始请求体（也可用 `--file FILE` 或管道 stdin）         |
| `a[b][c]=v`、`a[]=v`、`a[1]=v`、`[]:=1`                    | 嵌套 JSON 构建                           |

### 支持的参数速查

`-j/--json` `-f/--form` `--multipart` `--raw` `--file FILE` `-p/--print` `-h/--headers` `-b/--body`

`-m/--meta` `-v/--verbose` `-o/--output` `-d/--download` `-q/--quiet` `--pretty`

`-a/--auth` `-A/--auth-type` `--proxy` `-F/--follow` `--max-redirects` `--timeout`

`--check-status` `--offline` `--curl` `--verify` `-I/--ignore-stdin` `--default-scheme`

`--debug` `--help`

> 说明：http 子命令中
> `-h`
> 是 httpie 语义的 "只打印响应头"，因此帮助请用
> `sys http --help`
> 。

## 8. 快捷命令垫片（`short`）

### 特性



* 一键为**全部子命令**生成短命令垫片，之后任意目录直接使用

* Windows 生成 `.cmd` 垫片（`@echo off` + 转发），Ubuntu 生成可执行 `sh` 脚本（`chmod 755`），均转发全部参数

* 默认安装到托管 bin 目录（与 `link` 共用）并**自动持久化加入 PATH**；`--temporary` 只打印片段不写入

* `--dir` 指定目录、`-f/--force` 覆盖已存在的垫片

* 垫片转发 `%*` / `"$@"`，无参数数量限制，特殊字符安全



| 短命令     | 等价            |
| ------- | ------------- |
| `spath` | `sys path` |
| `senv`  | `sys env`  |
| `slink` | `sys link` |
| `shttp` | `sys http` |
| `sai`   | `sys ai`   |
| `stask` | `sys task` |
| `sfile` | `sys file` |

> 注：`data` / `dasel` 与 `log` 子命令**不生成垫片**（无 `sdata` / `slog`），按全名使用；`con` 子命令已删除（v0.5.10，由 `data` 取代），`search` 子命令已删除（v0.5.11）。

### 示例



```
sys short              # 安装全部 7 个垫片到托管目录并自动加入 PATH
sys short --dir ~/bin  # 指定安装目录
sys short -f           # 覆盖已存在的垫片
sys short --temporary  # 不持久化 PATH，只打印可粘贴的片段
```



***

## 9. 网络探测（`net`）

从文件中批量读取 http(s) 地址并逐条探测，判断可达性与响应格式。

### 特性

* `-i FILE`：逐行读取输入文件；以 `#` 开头（前导空白后判断）的行跳过；每行可含**多个地址**——地址从 `http`（大小写不敏感）起，一直截取到下一个空白字符为止，并修剪行尾标点（如 `http://a.com/x,` → `http://a.com/x`），scheme 统一为小写并校验 host

* **重复检测（所有模式）**：本次运行内已检测过的地址会缓存起来；再次出现同一地址（host 大小写、默认端口视为相同）**不重复检测、直接删除**——test 模式按删除管线处理（单地址行删整行 / 多地址行只删该地址），tv / file 模式不写入 `-o`，domain 模式不参与探测与排序；打印统一显示 `[DUP] <url> duplicate (already checked), skipped`，汇总行存在重复时追加 `, N duplicate`

* `-t domain`（默认）：对每个地址发起**连续 3 次** GET 请求，取平均响应时间；全部探测完毕后按平均响应时间**升序**打印，不可达地址列在末尾并附原因

* `-t tv`（旧名 `json`）：对每个地址发起 GET，响应体须为**有效的 tv JSON 数据**才算通过——能解析为 JSON（容忍尾随逗号、**BOM 前缀**等小瑕疵：先剥离 BOM、严格解析，失败则清理尾随逗号后重试），且 `sites` 为非空数组、每个元素都含 `name` 字段（`lives` / `parses` / `rules` 均可选）；通过后对数据统计并排序输出：
  * `--sort count`：统计 `.sites` 节点总数，按数量 **desc** 排序
  * `--sort name`：统计 `.sites.name` 中含关键词（默认 `4k`，大小写不敏感；用 `--words KEYWORD` 自定义，如 `--words hd`）的数量，按数量 **desc** 排序
  * `--sort speed`：下载 `.spider` 中的 url（字符串 `url;md5;...` 取 `;` 前部分，数组取首个字符串元素），按下载速度 **asc** 排序
  * `--sort` 可多个（如 `--sort count name`），按出现顺序多级排序；**不提供默认按 count, name 排序**
  * 每个通过的地址打印一行 url，下一行打印 `# count=<N> 4k=<N> speed=<速度或失败原因>` 测试信息（`4k` 为 `--words` 关键词本身，如 `--words hd` 时显示 `hd=<N>`）；**屏幕与 `-o` 文件格式一致**（url + `#` 行）；不可达 / 非 JSON 地址照旧打印 `[NO]` / `[FMT]`，不写入 `-o`

* `-t file`：对每个地址发起 GET，响应体必须是**普通文本**（UTF-8 非二进制，且不是 JSON）才算通过

* `-t url`：**离线提取**文件中的所有合法 http(s) URL，不做任何网络探测——沿用统一的输入约定（`#` 注释 / 空行跳过、每行可含多个地址、行尾标点修剪、scheme 统一小写并校验 host）；重复地址（host 大小写 / 默认端口视为相同）跳过并打印 `[DUP]`，每个唯一 URL 按首次出现顺序打印一行；`-o FILE` 写入提取出的唯一 URL 列表（一行一个）；文件不含 URL 时不报错（汇总 `0`，`-o` 写出空文件）

* `-t tvs`：继承 `-t tv` 全部功能，并**额外支持聚合型 JSON**（如 `{"urls":[{"url":"...","name":"..."}, ...]}`）。每个地址的响应体可为两种格式：
  * **单仓**：tv JSON（`sites` 非空数组、每元素含 `name`）→ 按 `--sort` 规则统计排序（count / name / speed，默认 count, name），`# count= name= speed=` 测试信息行
  * **多仓**：只有 `urls` 数组 → 只统计 `urls` 节点数（`# count=N`），按节点数 **desc** 排序
  * 两种格式都不是 → `[FMT]` 失败，不输出
  * 最终**分组输出**：单仓组（标题 `--- 单仓 (N) ---`）在前、多仓组（`--- 多仓 (N) ---`）在后，屏幕与 `-o` 文件格式一致

* `-t test`：只探测**可访问性**（收到任何 HTTP 响应即视为可访问，与 tv/file 口径一致）：
  * **每处理一行打印一行结果**，带进度计数 `[当前行/总行数]`（如 `[3/6]`，总行数含空行与注释行）：逐地址显示 `[OK] <url> time=...` 或 `[NO] <url> error=<原因>`，行尾标注 `-> kept` / `-> line deleted` / `-> kept (bad address removed)`；空行 / `#` 注释行 / 无地址行显示 `[skip]`
  * 不可访问的地址从源文件中删除：单地址行不可访问 → 删除整行；多地址行只删除其中不可访问的地址（保留可访问部分），整行全部不可访问才删除整行
  * 空行、`#` 注释行、不含地址的文本行一律保留
  * `-o FILE`：把**删除后的源文件结果**写入指定文件（保留的行按原顺序、原文本），一行一行
  * `-x SUFFIX`（可重复，如 `-x m3u -x zip`）：文件名后缀匹配（大小写不敏感，前导点可省略）的地址**不发起检测、直接删除**——例如 `-x m3u` 时 `https://www.main.com/123.m3u` 被直接删除；单地址行整体删除，多地址行只删该地址，打印 `[X] <url> excluded (-x m3u)`。仅在 `-t test` 模式生效，其他模式忽略并提示 warning

* `-t info`：通过 RDAP（`rdap.org` 引导）查询 `-n DOMAIN` 的**注册信息**——注册商 / 注册主体（注册人 registrant + 注册组织 org）/ 邮箱 / 地址 / **国家/地区（Country）** / 创建时间 / 到期时间 / 状态 / 域名服务器；若 registry 响应不返回注册主体，自动**跟随响应中的 related RDAP 链接**（ICANN profile：注册商级 RDAP 服务器携带完整实体，如 163.com → MarkMonitor → 广州网易计算机系统有限公司）；若该域名查不到注册数据，自动**逐层回退到其上层域名**（`a.b.example.com` → `b.example.com` → `example.com`），打印时标注 `no registration data, using parent ...`；`-n` 可重复（`-n a.com -n b.com`），此模式不需要 `-i`（`-o` / `-x` / `--sort` 忽略并 warning）

* tv / tvs 模式**多地址并行探测**：`--thread NUM` 指定一次并行测试多少个地址（默认 **CPU 内核数量**）；每个地址测完立即在屏幕打印一条 `[OK]` / `[FMT]` / `[NO]` 结果（含状态码与响应时间），全部完成后输出排序分组结果与汇总行

* tv / tvs 模式**行内备注**：输入行中地址后面（隔空格）出现 `#` 时，`#` 后的内容作为该地址的**备注**输出到测试结果中——逐条结果行末尾 `note=...`，最终排序/分组输出与 `-o` 文件的 `#` 测试信息行中显示 `note=备注  count=...`（备注取自该行最后一个地址；例如 `https://a/x.json  #二月红接口`）

* tv / file 模式**每测完一个立即打印**一条结果：URL / 是否可用 / 状态码 / 响应时间 / 实际格式（json / text / binary）

* `-o FILE`：把**检测成功**（地址可访问且格式符合 `-t` 要求）的 url 写入指定文件（仅 tv / file 模式有效）

* `--timeout SECONDS`：单次请求超时（默认 10 秒）

### 示例

```
# urls.txt 输入文件示例
# 注释行会被跳过
https://api.example.com/v1/data   # 第一个地址
https://www.example.com/readme.txt
见 https://a.com/x, 以及 https://b.com/y   # 一行可含多个地址
```

```
sys net -i urls.txt -t domain              # 3 次探测取平均，按平均响应时间升序打印
sys net -i urls.txt -t tv -o ok.json.txt   # JSON 统计排序输出（--sort count name），通过地址 + # 测试信息写入文件
sys net -i urls.txt -t tvs -o ok.groups.txt # 兼容 tv/聚合两种 JSON：单仓(.sites)/多仓(.urls) 分组输出，多仓按节点数降序
sys net -i urls.txt -t file -o ok.txt      # 只把返回普通文本的可用地址写入文件
sys net -i any.txt -t url -o urls.txt      # 离线提取文件中的全部合法 http(s) URL（去重），不发起任何探测
sys net -i urls.txt -t test -o clean.txt   # 探测可访问性，不可访问打印错误并从源文件删除，结果写入 clean.txt
sys net -i urls.txt -t tv --timeout 5      # 单次请求超时 5 秒
sys net -i urls.txt -t tv --thread 8        # tv/tvs 模式并行探测 8 个地址（默认 CPU 内核数）
sys net -i urls.txt -t tv -o ok.json.txt    # 输入行地址后的 "# 备注" 会随该地址输出（note=...）
sys net -t info -n www.example.com         # 查域名注册信息；查不到自动回退上层域名
sys net -t info -n a.com -n b.com          # 一次查多个域名
```

***

## 10. 日志查看（`log`）

彩色 less 式日志查看器：`sys log --file app.log [--level LEVEL] [--search KW] [--line N] [--newer TIME] [-f]`。解析每条日志的**时间**（多种常见格式）、**日志级别**与 **app 标志**并按语义着色；TTY 下进入全屏交互（同 less），管道 / 重定向输出纯文本（无 ANSI）。

### 特性

* **时间解析（橙色 RGB(255,165,0) 显示）**——以下格式均识别为日志开头：
  * ISO：`2024-01-15 10:23:45`、`2024-01-15T10:23:45.123Z`（毫秒可选）；Log4j 逗号毫秒：`2024-01-15 10:23:45,123`
  * 斜杠：`2024/01/15 10:23:46`
  * **nginx access log**：`15/Jan/2024:10:23:49 +0800`——时间在行**中间**也识别（行内前 80 字节扫描），前缀（如 `127.0.0.1 - - [`）完整保留
  * syslog：`Jan 15 10:23:48`（无年份取当年，落在未来 1 天以上自动回退上年）；US：`01/15/2024`
  * 带时区的时间按墙钟解释（`10:23:45 +0800` 显示为 `10:23:45+08:00`，不做 UTC 换算）

* **级别渐变着色**：`trace` 浅绿(178,230,178) → `debug` 绿(102,204,102) → `info` 绿(0,180,0)——**info 及以下为绿色，级别越低越浅**；`warning` 红(255,90,90) → `error` 深红(204,0,0) → `fatal` 最深红(140,0,0) 加粗。别名：`warn`==`warning`、`err`==`error`、`critical/crit/alert/emerg`==`fatal`

* **app 标志（深黄色 RGB(184,134,11)）**：时间后形如 `[http-nio-8080-exec-1]`、`app[1234]`、`[01-node-5]` 的标记

* **`--file FILE`**：指定日志文件（缺省读 stdin，`-` 亦为 stdin）

* **`--level LEVEL`**：按级别精确过滤（`trace | debug | info | warn | warning | error | fatal`）；无级别的行剔除

* **`--search KEYWORD`**：搜索关键字（大小写不敏感）——显示行号（灰色 RGB(130,130,130)），命中反显高亮；交互下 **n 跳下一条匹配 / u 跳上一条匹配**（循环，命中行居中）

* **`--line N`**：从第 N 行之后开始解析（保留原始行号）

* **`--newer TIME`**：只显示时间在指定时间之后的日志（无时间戳的延续行保留）

* **`-f`**：追踪文件变化，不断滚动展示——150ms 轮询增量读取，文件截断 / 轮转自动重读；仅文件可跟随（stdin 管道 + `-f` 报错退出码 1）

* **交互按键**（TTY 下）：`PageUp` / `PageDown` 翻页、`↑` / `↓` 逐行、`Home` / `End` / `g` / `G` 跳转、`q` / `Esc` / `Ctrl-C` 退出、`n` / `u` 搜索跳转；底部状态栏：` 文件名  L首-末/总数  level=..  "kw" i/N  following...  q=quit PgUp/PgDn=page ↑↓=line n/u=search`

### 示例

```
sys log --file access.log                       # 查看 nginx access log（时间在行中间也识别）
sys log --file app.log --level error            # 只看 ERROR（warn/error/fatal 均可）
sys log --file app.log --search "timeout"       # 搜索 + 行号，n/u 在命中间跳转
sys log --file app.log --level warn --line 100  # 从第 100 行之后开始
sys log --file app.log --newer "2024-01-15 10:23:47"   # 只显示该时间之后的日志
sys log --file app.log -f                       # 追踪文件变化滚动展示
type app.log | sys log --level error            # 管道输入（纯文本输出，无 ANSI）
```

***

## 11. 数据查询与转换（`data` / `dasel`）

查询、修改、转换结构化数据，dasel 兼容子集 + 扩展（原 `sys con` 的继承者，另加 TOML / table / jsonl 与函数表达式 / 切片 / 计数 / 投影）。别名 `dasel` 等同。

统一用法：`sys data [EXPR] [-f FILE | -u URL] [-i FMT] [-o FMT] [-w FILE] [--compact] [--root] [--depth [N]] [--limit N] [--header|--no-header]`。**EXPR 省略 = 纯格式转换**（如 `sys data -i yaml -o json < in.yaml`）。

### 11.1 输入来源与格式

* **输入来源**：stdin 管道（自动检测格式）→ `-f FILE`（按文件扩展名定格式，`-i` 覆盖）→ `-u URL`（GET 拉取，默认 json，`-i` 覆盖，如 `sys data -u https://api.dev/config status`）；三选一，`-f` 与 `-u` 互斥
* **七种格式互转**：`json` / `jsonl`（JSON Lines，`.ndjson` 同）/ `yaml`（`.yml`，tab 缩进自动归一）/ `toml` / `csv` / `md` / `table`。`-i` 定输入、`-o` 定输出（缺省 = 输入格式）；未知输入自动内容检测（整文 JSON → 每行 JSON → YAML → 逗号表 → table）
* **JSONL 语义**：每行一个独立 JSON 值，解析为**根数组**——选择器 / filter / `--limit` 全部按数组工作；`.log` 等文件内容检测兜底，无需显式 `-i`（如 `sys data -f app.log 'level=="error"'`）
* **CSV 与 table 的表头**：第一行默认即列标题（列名可直接查询）；第二行为 `----` 分隔线时识别为表头下划线并跳过（如 ps 输出的 `-------` 行）；`--header` 强制首行为列名、`--no-header` 显式启用 `col1..colN`（不自动推断）；table 行内多余列并入最后一列（COMMAND 含空格场景）、缺失列补空，单元格类型自动推断（数字 / 布尔 / 空 / 字符串）
* **非 UTF-8 自动解码**：文件与 stdin 按字节读取并宽容解码——UTF-8 BOM（剥离）→ UTF-16 LE/BE BOM → UTF-8 严格 → GBK → GB18030 → 宽松降级；GBK 设备日志可直接读（`专业` 等「GBK 恰似 UTF-8」陷阱按汉字回读纠正）
* **TTY 上色**：屏幕输出 json / yaml 按 jq 风格上色（key 蓝、value 绿，结构符保持默认）；管道 / 重定向 / `-w` 写文件一律纯文本，写文件内容永不带 ANSI；`NO_COLOR` 环境变量可关闭

### 11.2 选择器与切片

* **选择器语法**：点路径 `a.b`（前导 `.` 可省略）、数组索引 `list[0]` / `list[-1]`（负数从末尾数）、通配 `list[*]`、引号键 `m["中文键"]`（空格 / 点 / 中文键名）、根数组 `[0]` / `[-1]` / `[*]`（如 `ps | sys data -i table '[0].PID'`）
* **Python 式列表切片（只读）**：`list[:2]`（前 2 个）/ `list[1:3]` / `list[1:]` / `list[-2:]`（末尾 2 个）/ `list[::2]`（隔一取一）/ `list[::-1]`（反转）；省略边界取默认、负索引从末尾数、步长可负、step 0 报错；越界截断、空区间合法、`[0:99]` 全量。**只读约束**：可用于普通查询、`|count()` 管道（`data[:2]|count()` = 2）与投影左侧（`data[1:3]|{id,name}`）；`PATH=...` 写入与 filter / search / remove 路径明确拒绝
* 多段组合：`data[1:3][0]` 在切片结果上再取索引

### 11.3 修改表达式（赋值）

* `PATH = VALUE`：JSON 字面量（数字 / 布尔 / 对象 / 数组）或纯文本；**`= null` 删除**该键 / 元素
* **`PATH = @FILE`**：读取文件作为赋值值——扩展名定格式（json / jsonl / yaml / toml / csv / md），未知扩展名按 JSON → YAML → 纯文本兜底，如 `cat 4k.json | sys data 'wallpaper=@file.json' --root`
* **`PATH = strenv(NAME)`**：读取系统环境变量作为字符串赋值，变量未设置明确报错，如 `cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root`
* 算术更新：`PATH += N` / `-=` / `*=` / `/=` 数值运算（`+=` 对字符串做追加）
* **`-w FILE` 写回**：整份修改后的文档写入文件（表达式 + `--root` 时 stdout 安静）；**`--root`** 修改后打印整份文档而非选中值；**`--compact`** 单行 JSON 输出

### 11.4 函数表达式与谓词

* **`path.filter(谓词)`**：保留数组中满足谓词的元素（只读，输出结果数组）；**`path.remove(谓词)`**：原地删除满足谓词的数组元素（`--root` 打印整份修改后文档，否则输出剩余数组）；**`search(谓词)`**：从根（或指定节点）递归收集所有匹配节点（`$key` 为对象键 / 数组下标）
* **字符串 / 键函数**：`name.contains("x")` / `startsWith` / `endsWith` / `indexOf` / `has("k")`（对象键 / 数组下标存在性检查）
* **`count()` 节点计数**：统计匹配节点数——管道形式 `data|count()` / `data.filter(...)|count()` / `search(...)|count()`（数组计元素数、单值计 1、无匹配计 0）；点式 `data.count()`；裸 `count()` 计整个文档（数组 → 长度）。如 `ps | sys data -i table '[*]|count()'` 数行数
* **裸谓词自动过滤**：csv / table / jsonl 输入下，**比较表达式与方法谓词不加 `[*].filter` 直接可用**——`ps | sys data -i table 'Id==7360'`、`sys data -f app.log 'level=="error"'`、`'Name.contains("x")'` 等价于 `[*].filter(...)`；对象根文档上方法谓词仍是标量字符串函数
* **jq 式键投影管道**：`data[*]|{id,name,pricing}`——对每个匹配元素只保留列出的键（缺失键 → null），单匹配输出单个对象、多匹配输出数组；左侧可接 filter / search 结果（数组展开逐元素投影），如 `sys data 'clients.filter(name==''modelscope'')|{id,name}' -f config.yaml`；**裸 `{k1,k2}`（不带路径）直接作用于根文档**——对象根只保留这些键（jq `. | {k1,k2}`，如 `cat 4k.json | sys data '{wallpaper,warningText}'`），数组根逐行投影（同 `[*]`，如 `ps | sys data -i table '{Id,Name}'`）；键列表校验与管道形式共用同一函数，非法键（`{a.b}` 嵌套路径 / `{x: y}` 别名 / 空列表）同样报错；**深层节点尾随投影**：管道可省略——`sites[0]{key,name}` / `sites[0].{key,name}` 等价 `sites[0]|{key,name}`，切片同样可作左侧（`sites[:2]{key,name}`）
* **谓词语言**（dasel 兼容）：`$this` / `$key`、字段路径（`name`、`a.b[0]`、`m["中文键"]`）、字面量（数字 / 单双引号字符串 / true / false / null）、`== != > >= < <= && || !`、函数调用（`contains("x")` / `startsWith` / `endsWith` / `indexOf` / `has("k")`），示例 `sys data 'sites.filter(type==1)'`

### 11.5 文档级选项

* **`--depth N`**（默认 1，裸 `--depth` 即 1）：删除层级大于 N 的整棵子树（根 = 第 0 层，0..=N 保留），在表达式前应用——`sys data --depth 1 -f cfg.yaml` 只保留顶层键
* **`--limit N`**：文档根为数组时只保留前 N 个元素——csv / table 下即「只看前 N 条数据、表头仍输出」；jsonl 同；`--limit 0` 无输出（head -n 0 语义）；在表达式前应用，查询 / 过滤在截断后的文档上执行；对象根明确报错
* `-f` / `-u` / `-i` / `-o` / `-w` / `--header` / `--no-header` / `--compact` / `--root` 可任意组合；`--depth`、`--limit` 与选择器 / 赋值 / 函数 / 投影兼容

### 11.6 示例

```
echo '{"a":{"b":"你好"}}' | sys data 'a.b'            # 读取（stdin 自动检测 json）
sys data -f cfg.yaml 'server.port = 8080' -w cfg.yaml # 修改并写回
sys data -f cfg.yaml 'server.port = 8080' --root      # 修改并打印整份文档
sys data -i json -o yaml '{}'                          # 格式转换
sys data -i yaml -o json < in.yaml                     # YAML -> JSON
sys data -f app.log 'level=="error"'                  # jsonl 裸谓词过滤日志
sys data 'sites.filter(type==1)'                       # 过滤数组
sys data 'sites.remove(name.contains("web"))' --root   # 删除元素
sys data 'search(has("name"))'                         # 递归搜索
sys data 'name.endsWith("server")'                     # 字符串函数
sys data 'data[:2]|count()'                            # 切片 + 计数（= 2）
ps | sys data -i table '[*]|count()'                   # 数行数
ps | sys data -i table '[0].PID'                       # 根数组取首个元素字段
ps | sys data -i table 'Id==7360'                      # 裸谓词自动过滤（同 filter）
sys data 'data[*]|{id,name}' -f config.yaml           # jq 式键投影
cat 4k.json | sys data '{wallpaper,warningText}'       # 裸投影：对象根只保留这两个键
ps | sys data -i table '{Id,Name}'                     # 裸投影：数组根逐行投影（同 [*]|{...}）
sys data -f 4k.json 'sites[0]{key,name}'               # 深层节点尾随投影（== sites[0]|{key,name}）
sys data -f 4k.json 'sites[0].{key,name,type}'         # 点式写法等价
sys data -f 4k.json 'sites[:2]{key,name}'              # 切片 + 尾随投影：前 2 个站点逐元素
cat 4k.json | sys data 'wallpaper=@file.json' --root   # 文件赋值
cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root  # 环境变量赋值
sys data -u https://api.dev/config status              # URL 拉取 + 查询
sys data -i table --limit 3                            # 只输出前 3 行（表头保留）
sys data --depth 1 -f cfg.yaml                         # 只保留顶层键
sys data -f app.log '[*].filter(level=="error")' --limit 10  # 过滤 + 截断
```

> 注：PowerShell 5.1 调用原生命令会剥离内嵌双引号，谓词 / JSON 参数建议用单引号包裹（如 `'Id==7360'`、`'Name.contains(''x'')'`）或经 Python subprocess 传参；引擎本身同时支持 `'x'` 与 `"x"`。

***

## 限制



* `http`：`--auth-type digest`、`--session`、`--stream`、`--ssl`、`--cert`、自定义 `--boundary` 未实现（见 `sys help http`）

* `path export` 的 `.reg` 仅适用于 Windows（Registry Editor 格式），Linux 交换备份请用 `.txt` / `.json`

* `link` 符号链接在 Windows 上需要开发者模式或管理员权限，无权限时自动降级为拷贝

* `ai` 数据来自 models.dev 公开接口，离线时使用 24h 缓存，缓存过期且无网络时报告错误

## 开发与验证



* `cargo test`：Windows 26 个单测、Linux 23 个单测（平台相关用例按 `cfg` 门控）

* 已分别在 Windows（msvc）与 Ubuntu（WSL 实机）完成 `cargo check`、`cargo test`、`cargo build --release` 与端到端冒烟

* 端到端验证覆盖：PATH 增删改查与注册表恢复、链接回退链、环境变量持久化 / 临时模式、http 全参数冒烟、ai 真实联网抓取与 canonical 优选、short 五种垫片实际执行

* `dev/` 目录提供本地冒烟工具：`test_server.ps1`（HttpListener 测试服务器，端口 18899）、`smoke_http.ps1` / `smoke_http2.ps1`、`test_body.json`