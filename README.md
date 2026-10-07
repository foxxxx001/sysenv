# sys
[English](./README.en.md) | **简体中文**
跨平台（Windows / Ubuntu）Http接口客户端工具、AI模型查询工具、系统 PATH 与环境变量管理工具**，单一二进制、子命令划分功能。内置六大能力：



* **PATH 管理**：增删查改系统 PATH，自动持久化并即时作用于当前环境

* **注册表导入导出**：PATH 一键导出为注册表 / JSON / 文本，可反向合并或整体替换

* **链接到 PATH**：把任意文件挂进 PATH 目录，之后任意目录直接敲名字即可执行

* **环境变量管理**：读写系统环境变量，可选择持久化或仅临时生效

* **AI 模型查询**：从 models.dev 按名称爬取模型 / Provider 信息

* **联网搜索**：`sys search` 接入博查 AI 网页搜索 API（`api.bochaai.com/v1/web-search`），参数与官网接口一致；`--ai` 切换到 AI Search API（`/v1/ai-search`）返回 AI 答案与垂域模态卡

* **httpie 兼容 HTTP 客户端**：参数与 httpie 保持一致

* **快捷命令垫片**：一键生成 `spath / senv / slink / shttp / sai / ssearch / stask` 短命令



```
sys 0.4.13 (Made by Gary-china)

Usage: sys <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query the models.dev database of AI models & providers
  search Web search via the configured Bocha AI API (web-search; --ai uses ai-search)
  task   Query and kill processes (list PID/name/path; kill by PID or name)
  short  Install shell shims for every subcommand (spath/senv/slink/shttp/sai/ssearch/stask)
```



***

## 特性总览



| 功能        | 子命令                  | 核心特性                                                                                   |
| --------- | -------------------- | -------------------------------------------------------------------------------------- |
| PATH 增删查改 | `path`               | 持久化 + 当前会话即时生效；自动转绝对路径与去重；前置插入；user/machine 双作用域；`--temporary` 临时模式                    |
| 注册表导入导出   | `path export/import` | `.reg` / JSON / TXT 三种格式；合并或 `--replace` 整体替换；跨机迁移备份                                   |
| 链接到 PATH  | `link`               | 硬链接 → 符号链接 → 拷贝三级自动回退；Windows `.cmd` 垫片；自定义命令名；系统目录或托管目录                               |
| 环境变量      | `env`                | get /set/unset/list；默认持久化；`--temporary` 仅当前 shell；machine 作用域                          |
| AI 模型查询   | `ai`                 | `ai model`（models.dev：226 个 Provider、8000+ 模型；24h 缓存；canonical 优选；无参列出全部；`--date` / `--open` / `--model-type`（支持逗号多值取交集）/ `--search / --list / --json / --refresh`）；`ai cn-model`（datalearner：1015 个中文模型；`--date` / `--model-type` 过滤；精确命中附带详情页上下文长度与模态）；`ai info`（本地配置查询：`info provider` 列出 name/api_base/api_key、`info model` 列出 `{provider}:{name}` 模型清单、`info price` 查 models.dev 模型价格、`info balance` 用 api_key 查官方余额、`info sale-price` 抓官网定价页）；`ai config`（导出配置为 codex / opencode / litellm / freellmapi 格式，支持 `provider:*` / `provider:model` 选择与 `-f` 写文件）；`ai chat`（多 Provider 动态加权轮询聊天，`-m` 覆盖模型、`--list-model / --list-provider`、`--server` 以 OpenAI 兼容 API 对外提供 `/v1/chat/completions` 与 `/v1/models` 服务（默认 127.0.0.1:10000，可只给端口/IP），请求前自动补齐 max_input_tokens/type 并截断超长消息，OpenAI / Anthropic 兼容）；`ai image`（OpenAI 兼容 Images API `POST /v1/images/generations` 生成图片：`-o` 保存目录 / `-n` 数量 / `-s` 尺寸 / `--url` 下载 URL 版；b64 解码保存，文件头识别 png/jpg/gif/webp，多模型加权轮询与权重奖惩同 chat）；`ai task`（**无 -t 带输入时 function_call 自动路由**：任务 name 作函数名、desc 作函数描述，模型选任务后自动执行；`-t NAME` 模板聊天；`-t *` 全任务 0-10 分打分匹配） |
| 进程管理      | `task`               | 无参列出全部进程；查询（PID / 名称 / 路径，名称模糊匹配）；`-o` 查端口占用进程；按 PID 或名称终止；`-f` 强制 |
| 文本搜索替换  | `file`               | fd/sd 风格：1 参搜 stdin（带扩展名如 `me.txt` 则显示文件，引号包裹强制搜索）；2 参目录树文本/源码搜索（`-e` / `-i` / `-t` / `-w` / `-c`）；3 参 OLD NEW PATH 就地替换；`-S` 大小 / `--newer` / `--older` 时间 / `-d` 深度筛选，无 PATTERN 时列文件 |
| 格式转换      | `con`                | json / csv / md / yaml 互转：默认读 stdin（`cat a.json | sys con`），`-file` 读文件，`-i` 输入格式，`-o` 输出格式，`-out` 写文件；表格类转对象数组（类型推断 + 转义） |
| 联网搜索      | `search`             | 博查 AI 网页搜索 API（`POST api.bochaai.com/v1/web-search`）：`--freshness`（时间过滤）/ `--summary`（AI 摘要）/ `--count`（1-50）/ `--page` / `--include-domains` / `--exclude-domains`（域名白黑名单），参数与官网接口一致；key 取自配置 `search` 段；`--json` 原始响应 / `--debug` 请求与响应。`--ai` 切换到 AI Search API（`/v1/ai-search`）返回 AI 答案与垂域模态卡（`--no-answer` 关闭 AI 答案） |
| HTTP 客户端  | `http`               | httpie 参数子集对齐；**默认 application/json**（`-f`/`--multipart`/显式头可覆盖）；JSON / 表单 /multipart/ 原始体；嵌套 JSON；下载 / 重定向 / 认证 / 离线模式；`--help` 参数说明与示例；`--debug` 打印实际请求与响应（含头） |
| 快捷垫片      | `short`              | 一键安装七种短命令（spath/senv/slink/shttp/sai/ssearch/stask）；Windows `.cmd` / Linux sh 脚本；自动加入 PATH                                       |



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

* 无论哪种平台，`path add`、`env set` 都会**同时更新当前进程环境**（Linux 下通过向父 shell 输出可粘贴的 export 片段，Windows 下通过注册表广播），保证 "持久化 + 当前生效" 双满足。

* 所有写操作默认持久化；加 `--temporary` 则只打印可粘贴的 shell 片段（`export` / `$env:`），不写入任何持久化存储。



***

## 1. PATH 管理（`path`）

### 特性



* `list`：列出当前生效的完整 PATH（合并了进程与持久化作用域）

* `add`：追加到末尾；`-p/--prepend` 插入到开头；自动转绝对路径、大小写不敏感去重、跳过空条目

* `remove`：按绝对路径匹配移除（任意作用域）

* `has`：同时检查当前进程与已持久化作用域，报告该目录是否在 PATH 中

* `export / import`：注册表文件（`.reg`）、JSON（`.json`）、纯文本（`.txt`）三种格式双向转换

* `--scope user|machine`：控制写入作用域

* `--temporary`：不持久化，仅打印可粘贴的 PATH 片段

### 示例



```
sys path list                      # 列出当前生效 PATH
sys path add D:\tools              # 追加（自动转绝对路径、去重）
sys path add D:\tools -p           # 前置
sys path add /opt/bin --scope machine   # 机器级（需管理员/root）
sys path add D:\tools --temporary # 仅当前会话（打印可粘贴片段）
sys path remove D:\tools
sys path has D:\tools              # 同时检查当前进程与已持久化作用域
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

* `-o/--output-format json|csv`：机器可读输出 ——`json` 输出 **JSON 数组**（与 `--json` 等价，单命中也是数组）；`csv` 输出带表头的 CSV 表格（model 24 列、provider 6 列，RFC-4180 转义），适用于详情、搜索、列表与多匹配全部场景

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
sys ai model gpt-4.1 --refresh        # 强制重抓数据（--refresh 属 ai 层，也可写 ai --refresh model gpt-4.1）
sys ai provider openai                # Provider 详情（api / npm / models 数量与清单）
sys ai provider -s groq               # Provider 子串搜索
sys ai provider --list                # 列出全部 Provider
```

CSV 列：model 为 `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`；provider 为 `id,name,api,env,npm,models_count`。

#### 5.1 国内 AI 模型查询（`ai cn-model`）

数据源为 [datalearner](https://www.datalearner.com/ai-models/pretrained-models) 的预训练模型列表（服务端渲染 HTML，约 **1015 个模型**，分页抓取 + slug 去重），与 `ai model`（models.dev）相互独立。

* 只做查询：支持按**名称精确查询**与 `-s/--search` 子串搜索（别名也会匹配），**没有** `--list`、**没有** `--open`（该数据源不提供开放权重信息）

* `--date YYYY-MM-DD`：以卡片上的 **published 发布日期**为筛选标准，只显示**晚于**该日期的模型；单独使用 `--date`（不带名称 / 搜索词）时直接列出全部晚于该日期的模型

* `--model-type TYPE[,TYPE...]`：按**分类**筛选模型，取值 `text / image / audio / video / multimodal`（也接受中文 `文本 / 图像 / 语音 / 视频 / 多模态`）；**多个值用半角 `,` 或全角 `，` 分隔，取交集**（分类须同时匹配每个类型的关键词，如 `audio` 匹配“语音”分类、`image` 匹配“视觉 / 多模态”类）

* **精确命中时抓详情页**：单模型精确查询会额外抓取该模型的详情页，输出中附带 `context`（上下文长度，如 `1.05M`）与 `modality`（输入/输出模态，如 `文本、图像 → 文本`），失败时静默降级（字段留空）

* 字段：`id（slug）/ name / provider（发布机构）/ aliases（又名）/ type（精选 / 预览版 / 开源模型 / 闭源模型等徽章）/ category（分类，如 推理大模型）/ context（上下文长度，详情页）/ modality（输入/输出模态，详情页）/ published / url`

* 24 小时本地缓存（与 `ai model` 同一缓存目录，文件名 `datalearner-models.json`）；`--refresh` 强制重新抓取

* `--limit N` 限制列表条数（默认 20）；`--json` / `-o json` 输出 JSON 数组；`-o csv` 输出 10 列 CSV（`id,name,provider,aliases,type,category,context,modality,published,url`）

```
sys ai cn-model gpt-6-1-sol            # 精确查询（大小写不敏感；附带详情页 context / modality）
sys ai cn-model -s ernie --limit 10    # 子串搜索（别名也匹配）
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

* `clients` 列表存放 Provider：必填 `name / api_base / api_key / models`（`models` 每项 `name` + 可选 `weight`，缺省权重 1，**取值范围 0-9**，可选 `max_tokens`、`max_input_tokens`、`type`）；`type` 为 `openai`（默认，兼容 `open`）或 `anthropic`，分别按 OpenAI / Claude API 标准发请求

* **请求前自动补齐能力字段**：发送 HTTP 请求前，若模型项缺少 `max_input_tokens` 或 `type`，自动从 models.dev 按模型名称查匹配的第 1 个模型：`max_input_tokens` 用其 `context`（上下文长度）填充，`type` 用其输入/输出模态并集填充，然后**写回配置文件**（下次直接使用，不再查询）；models.dev 查询失败或无匹配时静默跳过，不阻塞请求

* **消息长度截断**：发送前检查消息字符数是否超过该模型的 `max_input_tokens`，超过则截取到限制内（stderr 打印截断提示），防止超出上下文窗口

* 顶层 `model` 选择模型，规则：
  - 缺失 → 第一个 Provider 的第 1 个模型
  - `provider:model` → Provider 名称与模型名称都匹配的那个模型
  - `provider:*` → 该 Provider 下**所有**模型，按 `weight` **带权重轮询**（缺省 1）
  - `model`（裸名）→ 所有 Provider 中名称匹配的模型合集，同样带权重轮询
  - **多个选择**：顶层 `model` 与 `-m/--model` 都可用半角逗号 `,` 或全角逗号 `，` 分隔多个选择（如 `agnes:*,claude:claude-3-5-sonnet`），按权重轮询使用

* `-m/--model MODEL`：**临时覆盖**顶层 `model`，规则与配置完全一致（`provider:model` / `provider:*` / 裸名 / 逗号分隔多选）

* `--list-provider`：列出配置中所有 Provider（名称 / type / 模型数）后退出；`--list-model`：按 Provider 分组列出所有模型（含权重）后退出；两者可同时使用，均无需消息

* **动态权重轮询**：请求失败时若该模型 `weight > 1` 则 `-1` 并**写回配置文件**，随后自动尝试下一个模型；替补模型成功且 `weight < 9` 则 `+1` 并写回。权重越高的模型被选中概率越大，失败惩罚 / 成功奖励持续生效（weight 0 表示基本不参与轮询）

* 轮询状态持久化在配置同目录的 `chat_state.json`，多次调用会持续轮转

* 消息来源：命令行参数（多段自动拼接）；无参数时若 stdin 非终端则读取管道内容

* 顶层 `stream: true` 时默认 SSE 流式输出（逐字打印）；`--no-stream` 关闭；`--debug` 时自动改为非流式

* `--debug`：把**实际 HTTP 请求**（方法 / URL / 请求头 / 请求体）与**响应**（状态 / 响应头 / 响应体）打印到 stderr，不污染 stdout

```
sys ai chat "你好"                     # 用默认配置聊天（~/.sysenv/config.yaml）
sys ai chat 你好 世界                  # 多参数自动拼接
echo "帮我总结这段文字" | sys ai chat   # stdin 管道
sys ai chat "你好" -c doc/config.yaml  # 指定配置文件
sys ai chat "你好" --no-stream         # 关闭流式
sys ai chat "你好" --debug             # 打印实际请求与响应（含 header）
sys ai chat "你好" -m agnes:agnes-3.0-flash   # 覆盖顶层 model，只用 agnes 的该模型
sys ai chat "你好" -m "agnes:*,claude:claude-3-5-sonnet"  # 多模型逗号分隔，加权轮询
sys ai chat --list-provider            # 列出配置中的 Provider
sys ai chat --list-model               # 列出配置中的所有模型（含权重）
sys ai chat --server                   # 以 OpenAI 兼容 API 对外提供聊天服务（默认 127.0.0.1:10000）
sys ai chat --server 8080              # 只给端口，host 用默认 127.0.0.1
sys ai chat --server 0.0.0.0           # 只给 IP，端口用默认 10000（对外网开放）
sys ai chat --server 0.0.0.0:9000      # 同时指定 IP 与端口
```

* `--server [ADDR]`：**把配置中的 Provider 以 OpenAI 兼容格式对外提供**（本地 HTTP 服务，默认 `127.0.0.1:10000`）。提供 `POST /v1/chat/completions`（非流式与 SSE 流式透传到上游 Provider）与 `GET /v1/models`（列出全部已配置模型，ID 为 `{provider}:{model}`）。请求中的 `model` 使用与聊天相同的选择规则（`provider:model` / `provider:*` / 裸模型名 / `auto` = 配置默认）。仅支持 `type: openai` 的 Provider，anthropic 目标返回明确错误。地址可只给端口（如 `8080`，host 用默认）或只给 IP（如 `0.0.0.0`，端口用默认）。

#### 5.3 任务模板聊天（`ai task`）

把 `tasks` 里预定义的模板组装成聊天消息后走 `ai chat` 通道。

* **无参数**：列出 `tasks` 中每个任务的 `name / desc`（最多 10 个）

* **带用户请求（无 `-t`）**：**function_call 自动路由** —— 每个任务的 `name` 作为函数名、`desc` 作为函数描述注册为 tools，大模型用 function_call 选择最匹配的任务（参数 `input` 携带用户请求），随后**自动执行**：把该任务 `desc` + 用户请求组装成消息发给模型，输出最终回复；模型未选择任何任务时明确报错（可改用 `-t NAME` 手动指定）

* **任务工具执行（function_call 选中后回填真实数据）**：任务可声明以下两种工具之一，选中后由**程序代码**（reqwest HTTP 请求，**禁止本地 shell 命令 / curl**）执行，执行结果作为"工具执行结果"回填给模型，模型再基于真实数据回答：
  * `api: <URL模板>` —— **固定 HTTP 接口**：`{key}` / `{key:默认值}` 占位符由模型 tool 参数填充（模型未传或传空时用默认值），响应体原样作为工具结果；`params: [k1, k2]` 声明参数名（可选，会加入函数 schema，便于模型填写坐标等参数）
  * `search: <源名>` —— **AI 模糊联网搜索**：内置搜索源由程序抓取头条列表回填，模型据此筛选总结。
    * **固定直连源**：`bilibili`（B 站热门视频）、`github`（近 7 天新建星榜，可选 `date` 参数如 `date:2026-01-01`）、`hn`（Hacker News 头条）、`toutiao`（今日头条热榜）、`oschina`（开源中国技术资讯）、`smzdm`（什么值得买今日好价，含价格）
    * **AI 聚合搜索源**（目标站点数据接口带签名/WAF，程序改用搜索引擎抓取相关结果回填，Sogou 优先、Bing 兜底，模型如实筛选总结）：`enlightent`（电视剧热播榜）、`dongchedi`（懂车帝汽车资讯）、`autohome`（汽车之家汽车新闻）；`sogou` 为通用聚合源，需传 `q` / `query` 参数（如 `q:电视剧 热播 榜单`）
    * **直连官方接口源**（v0.4.10）：`szhousing` —— 深圳楼盘销售情况。平台 API（`/szfdcscjy/*`）不经过瑞数反爬（瑞数只挂 HTML 首页），程序**纯 HTTP 直查官方接口，无需浏览器、不依赖本机任何软件**（Windows/Ubuntu 一致）。`name` 参数为楼盘名称（可选，如 `name:星悦尊府`），`area` 参数按区域查询（可选，如 `area:龙华` 或 `-area 龙华`），均未提供时返回近期在售项目列表
    * **博查 API 搜索源**（v0.4.12）：`bochaai` —— 博查 AI 网页搜索（`POST api.bochaai.com/v1/web-search`）。key 取自配置顶层 `search:` 段（如 `- name: bochaai, key: sk-...`）；`q` / `query` 为查询词（必填），可选参数与官网接口一致：`freshness`（`oneDay`/`oneWeek`/`oneMonth`/`oneYear`/`noLimit`/日期/区间）、`summary`（`true`/`false`）、`count`（1-50）、`page`、`include_domains` / `exclude_domains`（逗号分隔多域名）
    * **搜索源清单**：`sys ai task --list-source` 列出全部内置搜索源的名称、类型（`api` 直连数据接口 / `search` AI 聚合搜索 Sogou 优先 Bing 兜底 / `generic` 通用搜索需 `q`/`query` 参数）、用途与访问地址（v0.4.10）
    * **结果净化**（v0.4.9）：AI 聚合搜索自动过滤百度百科、搜狗百科、维基百科、互动百科等低价值词条（标题与 URL 双通道识别），避免搜索结果被百科词条占满；Sogou 请求失败时自动降级 Bing，两引擎均无可过滤结果时合并原始结果兜底，保证任务不因过滤而失败

* `-t <name>`：取 `name` 匹配的任务，按 `msg` 组装消息（`desc` 为任务描述）；任务声明了 `api`/`search` 时同样先执行工具并回填真实数据再回答

* `-t *`（**全任务打分匹配**）：把**所有任务**的 `desc` 与用户提供的聊天信息（命令行参数或 stdin）组装，调用配置的大模型按 **10 分制**打分（0=不匹配，10=完全匹配），输出 `TASK / DESC / SCORE` 表格并按分数降序排列；某个任务请求失败时其 SCORE 显示 `-` 并在 stderr 提示

* `msg` 模板占位符 `{key:默认值}`：命令行传 `key:值` 或 `key=值` 则替换为传入值，否则用默认值（如 `{country:深圳}` + `country:北京` → 北京）

* `msg` 前缀：`file://` 读取本地**相对路径**文件内容作为消息；`url:` 抓取网络内容作为消息

```
sys ai task                          # 列出任务（name / desc，最多 10 个）
sys ai task 今天深圳的天气如何         # function_call 自动路由：模型选 weather 并自动执行 api 工具（真实天气）后回答
sys ai task 今天有什么热门新闻        # function_call 自动路由：模型选 hotnews 并自动执行 toutiao 搜索源后回答
sys ai task -t weather               # 用默认值（深圳）组装消息并聊天
sys ai task -t weather country:北京   # 替换 country 为北京
sys ai task -t weather country=北京   # = 号写法等价
sys ai task -t "*" "帮我查今天深圳的天气"  # 对全部任务打分（TASK/DESC/SCORE，按分数排序）
```

**工具执行示例**（`~/.sysenv/config.yaml` 中 `tasks` 配置）：

```yaml
tasks:
  # 固定 HTTP 接口：占位符 {lat}/{lon} 由模型参数填充，未传时用默认值（深圳坐标）
  - name: weather
    desc: 用来获取天气信息
    msg: 我在{country:深圳},今天的天气如何，我要询问温度、湿度、下雨概率等信息
    api: https://api.open-meteo.com/v1/forecast?latitude={lat:22.54}&longitude={lon:114.06}&current_weather=true
    params: [lat, lon]
  # —— v0.4.5 新增搜索源 ——
  # 固定直连源：toutiao（今日头条热榜 JSON）、oschina（开源中国资讯 SSR）、smzdm（什么值得买好价 SSR）
  - name: hotnews
    desc: 获取今天的热门新闻（今日头条热榜）
    msg: 请基于工具执行结果列出今天的头条热门新闻，注明来源链接
    search: toutiao
  - name: oschina
    desc: 获取开源中国技术新闻资讯（oschina.net）
    msg: 请基于工具执行结果列出今天的技术新闻，注明来源链接
    search: oschina
  - name: deals
    desc: 获取今天什么值得买的特价商品（好价榜单）
    msg: 请基于工具执行结果列出今天值得买的特价商品与价格
    search: smzdm
  # AI 聚合搜索源（Sogou 优先、Bing 兜底）：目标站点的数据接口带签名/WAF，程序改用搜索引擎抓取相关结果回填，模型如实筛选总结
  - name: cloudrank
    desc: 获取热播电视剧榜单（云合数据霸屏榜/热播榜，AI 聚合搜索）
    msg: 请基于工具执行结果列出云合数据的霸屏榜/热播剧排名
    search: enlightent
  - name: auto
    desc: 获取汽车新闻资讯（汽车之家，AI 聚合搜索）
    msg: 请基于工具执行结果列出今天的汽车新闻资讯
    search: autohome
  - name: cartech
    desc: 获取汽车新闻资讯（懂车帝，AI 聚合搜索）
    msg: 请基于工具执行结果列出今天的汽车行业新闻与新车资讯
    search: dongchedi
  # —— v0.4.6 新增 ——
  - name: szhousing
    desc: 查询深圳楼盘销售情况（深圳房地产信息平台公开数据；提供楼盘名称 name 参数可查具体项目各楼栋销售状态统计，area 参数按区域查询如 area:龙华，未提供则返回在售项目列表）
    msg: 请基于工具执行结果说明深圳楼盘的房源销售/成交情况，注明来源链接
    search: szhousing
    params: [name, area]
```

实际输出示例（`sys ai task 今天深圳的天气如何`，模型选 weather → 程序请求 open-meteo → 回填后回答）：

```
[0] task: weather
    desc: 用来获取天气信息
    args: {"input":"今天深圳天气如何"}
→ 执行任务 weather
（工具执行结果：{"latitude":22.530754,"longitude":114.08714,"current_weather":{"temperature":27.2,...}}）
根据实时天气数据，今天深圳（坐标：22.53°N, 114.09°E）的天气情况如下：
- 温度：当前气温约为 27.2°C
- 天气状况：大致晴朗或多云（WMO code 1）
...
```

#### 5.4 AI 图片生成（`ai image`）

通过 Provider 的 OpenAI 兼容 **Images API**（`POST {api_base}/images/generations`，Bearer 鉴权）生成图片并保存到本地。

* 配置文件与 `ai chat` 完全一致（默认 `~/.sysenv/config.yaml`，`-c/--config` 覆盖），只支持 **OpenAI 兼容 Provider（`type: openai`）**；`type: anthropic` 目标在发请求前明确报错（Anthropic 没有 Images API）

* **模型选择与 `ai chat` 同一套规则**：顶层 `model` 或 `-m/--model`（`{provider}:{model}` / `{model}` / `provider:*` / 逗号分隔多选），多目标按 `weight` 加权轮询，失败自动降权并尝试下一个模型，替补成功升权，权重写回配置文件

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

#### 5.5 本地配置查询（`ai info`）

检查本地配置文件（默认 `~/.sysenv/config.yaml`，`-c/--config` 覆盖）与 models.dev 模型价格。

* `info provider [KEYWORD]`：列出 `clients` 中全部 provider 的 `name / api_base / api_key` 及官方 `DOCS`（帮助文档）与 `CONSOLE`（控制台）地址（按 provider 名称匹配内置收录表，未收录显示 `-`）；带 KEYWORD 时只保留名称包含该词的 provider（大小写不敏感），无匹配报错并列出可用名称；`-o json` / `-o csv` / `--json` 输出 JSON 数组或 CSV 表（JSON 字段 `name` / `api_base` / `api_key` / `docs` / `console`）
* `info model [KEYWORD]`：列出全部 provider 下的 model，格式 `{provider}:{name}`；带 KEYWORD 时按 provider 名称包含匹配（支持 `provider:model` / `provider:*` 精确选择），无 provider 命中时回退按 model 名称匹配；`-o json` / `-o csv` / `--json` 输出 JSON 数组或 CSV 表（JSON 字段 `provider` / `name`）
* `info price P1,P2,...`：按逗号分隔（支持全角 `，`）的 provider 名称查询模型价格列表，输出 input / output / cache_read 每 1M token 价格（`$`），数据源 models.dev（id/名称均匹配，复用 24h 缓存，`--refresh` 强制刷新）
* `info balance PROVIDER`：用该 provider 的 `api_key` 到官方接口查询余额。minimax 走官方 `token_plan/remains`（Token Plan 剩余额度；无订阅时给出按量付费余额入口）；agnes 走 OpenAI 兼容 billing 探测（credit_grants / subscription / usage，无余额字段时提示控制台）；alibaba-cn（阿里云百炼）走官方 `models/limits`（Key 级用量限额/限流配额，可验证 Key；官方无 Key 级现金余额接口，提示到百炼控制台）；modelscope / sensenova / bigmodel / amd / anspire 官方未开放 Key 级余额接口，命令给出对应控制台地址。余额实时查询，不走缓存
* `info sale-price PROVIDER`：抓取官网定价页并列出全部模型销售价。agnes 抓取 `wiki.agnes-ai.cn` 定价页（文本/图片/视频模型，刊例价 + 现价，人民币）；minimax 抓取 `platform.minimaxi.com` 定价页（语言模型输入/输出/缓存价格，元/百万 tokens）；alibaba-cn 给出百炼官方模型列表与计费说明页地址；其余 provider 给出官网定价页地址。定价页 24h 缓存，`--refresh` 强制重抓

```
sys ai info provider                     # 列出配置中全部 provider（name/api_base/api_key）
sys ai info provider agnes               # 只显示名称包含 agnes 的 provider
sys ai info provider -o json             # 全部 provider 的 JSON 数组输出
sys ai info provider agnes -o csv        # 过滤 + CSV 输出
sys ai info provider --json              # 等价于 -o json
sys ai info model                        # 列出全部模型，格式 provider:model
sys ai info model modelscope             # 只显示 modelscope 下的模型
sys ai info model deepseek               # 无 provider 命中时按模型名匹配
sys ai info model agnes:3.0              # 精确选择 agnes 下名称含 3.0 的模型
sys ai info model modelscope:*           # modelscope 全部模型
sys ai info price openai,anthropic       # 查询两个 provider 的模型价格列表
sys ai info price "openai，deepseek"     # 全角逗号同样支持
sys ai info balance minimax              # 用 minimax 的 api_key 查询官方余额
sys ai info balance alibaba-cn           # 阿里云百炼：官方 limits 限额 + 控制台指引
sys ai info sale-price agnes             # 抓取 agnes 官网定价页，列出全部模型销售价
```

#### 5.6 配置导出（`ai config`）

把本地配置（`~/.sysenv/config.yaml` 的 `clients` 段）导出为其他 AI 工具的配置格式：

* `ai config FORMAT [SELECT] [-f FILE] [-c FILE]`，`FORMAT` 支持：
  * **`codex`**（TOML）：每个 Provider 一个 `[model_providers.<name>]` 段（`name` / `base_url` / `env_key` / `wire_api = "chat"`）；codex 只从环境变量读 key，导出时生成 `SYS_<PROVIDER>_API_KEY` 命名并打印 `export` 提示到 stderr；顶层 `model` 选中第一个模型的 `provider.model`
  * **`opencode`**（JSON）：每个 Provider 一个条目（`npm: "@ai-sdk/openai-compatible"` + `options.baseURL` / `options.apiKey` 内联 + `models` 映射），可直接合并进 `opencode.json`
  * **`litellm`**（YAML）：每个模型一条 `model_list` 条目，暴露名 `{provider}:{model}`（防冲突），路由到 `openai/{model}`，`api_base` / `api_key` 内联
  * **`freellmapi`**（JSON）：`customProviders` 数组，每个 Provider 一个 `baseUrl` / `label` / `models` 条目（`supportsTools: true`），可合并进 `freellmapi.config.json`
* `SELECT` 选择导出范围：缺省 = **全部 Provider 的全部模型**；`provider:*` = 指定 Provider 的全部模型；`provider:model` = 指定 Provider 的单个模型（无匹配明确报错）
* `-f/--file FILE` 写入文件（缺省打印到 stdout）；`-c/--config` 指定配置文件

```
sys ai config codex                      # 全部模型导出为 codex TOML（stdout）
sys ai config opencode -f opencode.json  # 写入文件
sys ai config litellm "agnes:*"          # 只导出 agnes 的模型
sys ai config freellmapi "minimax:MiniMax-M2.7"   # 只导出 minimax 的单个模型
```

#### 5.7 网络搜索（`search`）

通过**博查 AI 网页搜索 API**（`POST https://api.bochaai.com/v1/web-search`，Bearer 鉴权）提供联网搜索能力，**请求参数与官网接口完全一致**。

* **API key**：读取配置顶层 `search:` 段（默认 `~/.sysenv/config.yaml`，`-c/--config` 覆盖），首个 `name: bochaai` 条目生效，未配置时明确报错。示例：

  ```yaml
  search:
    - name: bochaai
      key: sk-xxxxx
  ```

* **参数（与官网一致）**：
  * `query`：搜索词（位置参数，多词自动空格连接；无参数且 stdin 为管道时读取 stdin）
  * `--ai`：切换到 **AI Search API**（`POST /v1/ai-search`）——在网页搜索基础上额外返回垂域结构化模态卡（天气/百科/日历/股票等）与 **AI 实时生成的答案**（`answer` 默认开启；`--no-answer` 关闭，仅可与 `--ai` 同用；`stream` 固定关闭）
  * `--freshness <VALUE>`：时间过滤 —— `noLimit`（默认）/ `oneDay` / `oneWeek` / `oneMonth` / `oneYear` / `YYYY-MM-DD` / `YYYY-MM-DD..YYYY-MM-DD`
  * `--summary`：Web Search 模式下返回 AI 摘要并附带每条结果的摘要 / 站点 / 发布时间（AI Search 默认附带）
  * `--count <N>`：返回条数（1-50，默认 10）
  * `--page <N>`：页码（默认 1）
  * `--include-domains <D>`：只返回这些域名的结果（可重复）
  * `--exclude-domains <D>`：排除这些域名的结果（可重复）
  * `--json`：打印原始 JSON 响应；`--debug`：打印实际 HTTP 请求与响应

```
sys search "2026年诺贝尔物理学奖"                     # 默认 10 条，标题 + 链接
sys search 深圳 今天 天气 --summary --count 5        # 5 条并带 AI 摘要 / 站点 / 时间
sys search "rust 2026" --freshness oneMonth          # 只搜最近一个月
sys search "cargo 教程" --include-domains rust-lang.org docs.rs   # 只从指定站点搜
sys search "期货 行情" --exclude-domains baidu.com --json          # 排除站点 + 原始 JSON
echo "今天有什么热门新闻" | sys search               # stdin 管道查询
sys search "杭州天气" --ai                           # AI Search：AI 答案 + 天气卡 + 参考网页
sys search "深圳 买房 政策" --ai --no-answer --count 5   # AI Search 但不要 AI 答案
ssearch "五一 放假 安排"                                 # ssearch 垫片等价于 sys search
```

> 注意：**AI Search 与 Web Search 是博查的独立套餐**，key 需在开放平台分别开通；未开通 AI Search 包时 `--ai` 会返回 403（`You do not have enough money or package quota`）。

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

文件属性筛选（fd 风格，可与搜索/替换/列文件叠加）：

* `-S SIZE` / `--size SIZE`：只处理大小达到指定值的文件。纯数字按字节；`2k`/`2m`/`2g`/`2t`（或 kb/mb/gb/tb）按 1024 进制，支持小数（`1.5m`），大小写不敏感
* `--newer TIME`：只处理修改时间不早于 TIME 的文件；`--older TIME`：只处理早于 TIME 的文件。时间格式 `YYYY-MM-DD [HH:MM[:SS]]`（`T` 或斜杠分隔也可），按本地时区解释
* `-d NUM`：只递归 NUM 级子目录（`-d 0` 仅当前目录）

给出任一筛选且不带 PATTERN 时进入**列文件模式**（每行一个路径，默认当前目录，`-e`/`-t` 可附加筛选）；带 PATTERN 或 OLD NEW 时筛选叠加到字符串搜索与就地替换。

遍历规则：递归目录树时跳过隐藏项（`.` 开头）与常见噪音目录（`.git` / `node_modules` / `target` / `dist` / `build` / `__pycache__` 等）；二进制文件（含 NUL 字节）自动跳过；替换模式同样遵守 `-e` / `-t` / `-i` / `-w`。

```
sys file hello                              # stdin 管道搜索（type a.txt | sys file hello）
sys file me.txt                             # 显示 me.txt 文件内容
sys file "me.txt"                           # 引号包裹：仍按字符串搜索（stdin 中找 me.txt）
sys file hello D:\projects                  # 在 D:\projects 树中搜索
sys file hello D:\projects -e py -e md      # 只搜 .py 和 .md
sys file HELLO D:\projects -i               # 忽略大小写
sys file hello D:\projects -t               # 只搜纯文本（不含源码）
sys file hello D:\projects -w -c 2          # 整词 + 前后 2 行上下文
sys file hello hi D:\projects               # 就地替换 hello -> hi
sys file HELLO hi D:\projects -i            # 忽略大小写替换
sys file -S 2m                              # 列出当前目录下 >= 2 MiB 的文件
sys file -S 5000 D:\data                    # 列出 D:\data 下 >= 5000 字节的文件
sys file -S 2m -e bin -d 1 D:\data          # >= 2MiB 的 .bin 文件，只递归 1 层
sys file --newer "2026-10-01" D:\data       # 修改时间 >= 2026-10-01 的文件
sys file --older "2026-10-01 12:00" D:\data # 修改时间早于该时刻的文件
sys file hello D:\data -S 1m                # 只在 >= 1 MiB 的文件中搜 hello
```

#### 5.9 格式转换（`con`）

json / csv / md / yaml 四种格式互转。默认从标准输入读取（管道用法：`cat a.json | sys con`），结果输出到 stdout；`-out` 可改写到文件。

* `-file F`：从文件 F 读取（省略时读 stdin；扩展名为 json/csv/md/yaml 时自动推断输入格式）
* `-i FMT`：输入格式 `json | csv | md | yaml`（省略时按内容自动检测）
* `-o FMT`：输出格式 `json | csv | md | yaml`（省略时按输入格式输出，即仅格式化显示）
* `-out F`：把结果写入文件 F（默认只输出到 stdout）

表格类转换（csv / md）与对象数组互相映射：CSV 首行为表头；Markdown 表格解析表头 + 数据行。单元格自动做 null / bool / int / float / 字符串推断；字段含逗号、引号、换行、`|` 时自动转义。YAML 解析失败时回退到 tab-tolerant 解析器，兼容含 tab 缩进的真实配置。

```
cat a.json | sys con                        # stdin 自动检测，按原格式格式化输出
sys con -file a.json -o csv                 # json -> csv（stdout）
sys con -i csv -o json < a.csv              # csv -> json
sys con -file a.yaml -o md -out out.md      # yaml -> markdown，写入 out.md
sys con -file config.yaml -o json           # 真实配置（含 tab 缩进）-> json
type a.csv | sys con -o md                  # csv -> markdown 表格
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

* 网络行为：`-F/--follow` 跟随重定向、`--max-redirects`、`--timeout`、`--proxy`、`--verify no` 跳过证书校验、`--offline` 只构建并打印请求不发送、`-I/--ignore-stdin`

* `--check-status`：3xx → 退出码 3，4xx → 4，5xx → 5（脚本友好）

* `--help`：打印接口参数说明与示例（http 子命令的 `-h` 是 httpie 语义的“只打印响应头”）

* `--debug`：把**实际 HTTP 请求**（方法 / URL / 请求头 / 请求体，含 Content-Length）与**响应**（状态行 / 响应头 / 响应体）打印到 stderr，stdout 保持正常输出，便于排查真实发包内容

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

`--check-status` `--offline` `--verify` `-I/--ignore-stdin` `--default-scheme`

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

### 示例



```
sys short              # 安装全部 6 个垫片到托管目录并自动加入 PATH
sys short --dir ~/bin  # 指定安装目录
sys short -f           # 覆盖已存在的垫片
sys short --temporary  # 不持久化 PATH，只打印可粘贴的片段
```



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