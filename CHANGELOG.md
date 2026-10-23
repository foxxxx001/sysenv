# sys 更新日志（CHANGELOG）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sys-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

## v0.6.23（2026-10-23）

### 新增
- **`sys data` 赋值读取环境变量 `PATH = strenv(NAME)`**（src/dasel.rs `parse_env_value`）：右值为 `strenv(NAME)` 时读取系统环境变量 `NAME` 的值作为**字符串**赋值（名称两侧空白忽略），如 `cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root`；**未设置报错**（`environment variable <NAME> is not set`，不静默填空串）、非 UTF-8 值报 not valid UTF-8、`strenv()` 空名报 needs an environment variable name、名称含括号报 invalid `strenv(...)`
- **右值解析链**：`@FILE` → `strenv(NAME)` → 字面量，三者互斥且均不触发 `= null` 删除；`PATH += strenv(NAME)` 字符串拼接生效

### 修复
- **赋值表达式被误判为 unknown function**：`PATH = strenv(X)` / `PATH = foo(y)` 以 `)` 结尾会被 `parse_function_expr` 抢先当作函数调用报错。修复：`split_expression` 增加括号 / 方括号 / 花括号深度跟踪（括号内的 `=` 视为表达式内部、非赋值运算符，`list.filter(a = b)` 谓词不受影响），且 cmd_data 赋值分支提前到函数表达式分支之前

### 变更
- 版本 0.6.22 -> 0.6.23；README（中 / 英）data 特性行、main.rs `EXPR` 帮助文本、requirements.md 同步
- `cargo test --bin sys`：391 -> 394 项全绿（新增 `parse_env_value` 相关 3 项；`expression_splitting` 增加括号深度断言）

---

## v0.6.22（2026-10-23）

### 新增
- **`sys data` 赋值读取文件 `PATH = @FILE`**（src/dasel.rs `load_file_value`）：右值以未加引号 `@` 开头时读取并解析文件作为赋值，如 `cat 4k.json | sys data 'wallpaper=@file.json' --root`；扩展名定格式（json / jsonl / yaml / toml / csv / md），未知扩展名依次尝试 JSON → YAML → 纯文本字符串
- `@` 空报 `@` needs a file path；带引号 `"@x"` 与含 `@` 的多字面量不触发；`@` 值永不触发 `= null` 删除分支

### 变更
- 版本 0.6.21 -> 0.6.22；README（中 / 英）data 特性行、main.rs `EXPR` 帮助文本、requirements.md 同步
- `cargo test --bin sys`：386 -> 391 项全绿（新增 `load_file_value` 相关 5 项）

---

## v0.6.21（2026-10-23）

### 新增
- **`sys data` jq 式键投影 `path | {k1,k2}`**（src/dasel.rs `split_pipeline` / `parse_projection` / `project_values`）：`cat 4k.json | sys data 'data[*]|{id,name,pricing}'` 对每个匹配节点投影指定字段——单匹配输出对象、多匹配输出数组、缺键 → null；`{a.b}` / `{a[0]}` / `{x:y}` 报"暂不支持"
- **filter / search 后接投影**：`[*].filter(...)|{id,name}` 等价 jq 的 `.[] | select(...) | {...}`（函数结果数组展开逐元素投影；`apply_filter_value` / `apply_search_value` 拆出复用）；顶层 `|` 拆分跳过引号 / 括号 / `||`

### 变更
- 版本 0.6.20 -> 0.6.21；README（中 / 英）data 特性行、main.rs `EXPR` 帮助文本、requirements.md 同步
- `cargo test --bin sys`：379 -> 386 项全绿（新增 projection 相关 7 项）

---

## v0.6.20（2026-10-22）

### 新增
- **`sys ai --free[=ROUTER]` 内置免费模型路由**（src/chat.rs `free_config` / `builtin_free_config` / `parse_free_router`，src/ai.rs `cmd_info`）：无配置文件也可使用内置免费模型——`kilo-auto/free` 匿名直连（Kilo Gateway，keyless），`OPENROUTER_API_KEY` 已设置时追加 `openrouter/free`；`--free` 与 `--free=ROUTER`（kilo | openrouter，大小写不敏感，接受 `kilo-auto` / `kilo-auto/free` 等别名）固定路由；权重轮换状态仅存内存、不写回；配置文件缺失时自动启用；`sys ai chat` / `sys ai info providers` / `sys ai info models` 均支持
- **`sys task list --path/-p` 可执行路径过滤**（src/task.rs `cmd_list` / `proc_matches`）：对进程可执行路径做大小写不敏感子串匹配，与 NAME（子串 / 数字按 PID 精确）及 `-o/--port` 以 AND 组合；无匹配时按过滤维度给出针对性报错

### 变更
- 版本 0.6.18 -> 0.6.20（0.6.19 无独立发布，合并于此）；README（中 / 英）、main.rs 帮助文本、requirements.md 同步
- `cargo test --bin sys`：370 -> 379 项全绿（`--free` 路由与 `task list` 路径过滤相关 9 项）

---

## v0.6.18（2026-10-20）

### 新增
- **`sys net -t url`：从文件中离线提取合法 http(s) URL**（src/net.rs 新增 `NetType::Url` + `cmd_url`）：不做任何网络探测（`cmd_net_t` 在构建 HTTP client 前提前返回）——沿用统一输入约定（`#` 注释 / 空行跳过、每行多地址、行尾标点修剪、scheme 小写并校验 host）；重复地址跳过并打印 `[DUP]`，每个唯一 URL 按首次出现顺序打印一行；`-o FILE` 写入提取出的唯一 URL 列表（一行一个）；文件不含 URL 时不报错（汇总 0、`-o` 空文件）；`-x` / `--sort` 忽略并 warning

### 变更
- 版本 0.6.17 -> 0.6.18；README（中 / 英）`net` 段落 + 示例行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys`：366 -> 370 项全绿（新增 4 项：提取顺序与去重 / 注释与伪 URL 剔除 / scheme 规范化 + `-x`、`--sort` 忽略 / 无 URL 不报错）

---

## v0.6.15（2026-10-20）

### 修复
- **`sys ai chat --query-model` 的 models 接口地址规则修正**：OpenAI 兼容 provider 的模型列表地址不再假设为 `{api_base}/v1/models`，而是把 `models` 直接追加在配置的 api_base 后（`{api_base}/models`）——如百度千帆配置 `https://qianfan.baidubce.com/v2` 时正确请求 `/v2/models`（此前生成 `/v2/v1/models` 返回 404 ResourceNotFound）。Anthropic 类型保持 `{api_base}/v1/models`（api_base 已以 `/v1` 结尾时用 `{api_base}/models`）
- 规则收敛为单一实现：chat.rs `models_url` 改为 `pub(crate)`，ai.rs 删除私有副本并改调 `chat::models_url`，`--query-model` 与 `info list-model` 两条路径不再各自维护

### 变更
- **`sys ai info models` 与 `--query-model` 同功能**：`info models PROVIDER` 从"与 `info model` 重复的 models.dev 查询"改为查询该 provider 自身 models 接口的可用模型（等价 `info list-model PROVIDER` / `sys ai chat --query-model PROVIDER`，实时、分页翻页、`-o json/csv/md/yaml`）；`info model` 保持原配置模型清单查询
- 版本 0.6.14 -> 0.6.15；README（中 / 英）`ai info` 段落 + 特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys`：363 -> 364 项全绿（`models_url` 测试重写为 openai 直加 `/models`（含百度 `/v2` 用例）+ anthropic `/v1` 规则；新增 1 项：`info models` 无参报错、且与 `info model` 不再同义）

---

## v0.6.14（2026-10-19）

### 变更
- **以 api_base 判定"是否同一 Provider"**（导入 / 导出统一）：Provider 的身份是其端点，不再按名字归并
  - 导入（`merge_imported_providers` / `import_into_config`）：来源中指向同一 api_base（尾部斜杠与大小写忽略）的条目合并为一个块（首个条目的名字 / kind / api_key 保留，模型按名去重）；指向不同 api_base 的条目即使名字相同也是不同 Provider，各自成块。原"同名不同端点强制合并 + note"逻辑移除
  - 导出（`export_groups` + 新增 `export_key`）：8 种格式（codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder）按 api_base 归组——同一端点只输出一个条目/块（首个块名字 / api_key 生效，模型合并去重）；同名但端点不同的 Provider 保持为不同条目，条目名冲突时以 `名字@主机名` 唯一化（如 `openroute@openrouter.ai`）
  - 加载：重复检测同步改为按 api_base（同端点重复才 warning；同名不同端点视为不同 Provider，不再误报）
- 版本 0.6.13 -> 0.6.14；README（中 / 英）`ai config` 段落 + 特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys`：363 项全绿（导出 2 项重写为同端点归并 / 同名不同端点分开；导入 2 项重写为同端点合并 / 不同端点分块；加载 1 项重写为按 api_base 检测）

---

## v0.6.13（2026-10-19）

### 新增
- **导出同样按同名 Provider 归并**（src/ai.rs 新增 `export_groups`）：`sys ai config` 导出到 codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder 时，选中模型按 Provider 名（大小写不敏感）归组——同一 Provider 只输出一个条目/块（首个块的 api_base / api_key 生效，模型跨同名块合并、按模型名去重），**绝不为同一 Provider 建立多个条目**
- 修复两个由此暴露的隐患：litellm 同名 Provider 同名模型会输出重复 `model_name` 条目（现按名归并后天然去重）；freellmapi 同名 Provider 的后续块模型原本会被静默丢弃（现合并进同一条目）

### 变更
- 版本 0.6.12 -> 0.6.13；README（中 / 英）`ai config` 段落 + 特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys` 361 -> 363 项全绿（新增 2 项：同名 Provider 跨 8 格式导出一块/一条（含 freellmapi 模型合并、litellm 无重复）；同名大小写变体仍归并为一组）

---

## v0.6.12（2026-10-19）

### 新增
- **配置约定：同一 provider 的模型必须配置在同一块内**——config.yaml 的 `clients:` 列表不得出现同一 provider（同名，大小写不敏感）拆成多个条目
- **`ai config --import` 同名 Provider 强制归并**（src/ai.rs `merge_imported_providers`）：来源中同名的 Provider 一律合并进同一个块（首个条目的 api_base / api_key 保留），`models.json` 不再按「同名不同 api_base 加 `-2` 后缀」拆块（原行为移除），opencode / zcoder 多个 `provider.<id>` 配相同 `entry.name` 时也只写一个块；同名但端点不同时保留首个 api_base 并打印 `sys: note: ... merged into one block (kept ...)`；`import_into_config` 入口同样先归并再写入，**config.yaml 绝不写入同一 Provider 的多个条目**
- **配置加载重复 Provider 名 warning**（src/chat.rs `config_from_yaml`）：解析 `clients:` 时发现同名 Provider 打印 `sys: warning: duplicate provider name ...`（不阻断，两个块都保留）

### 变更
- 版本 0.6.11 -> 0.6.12；README（中 / 英）`ai config` 段落 + 特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys` 358 -> 361 项全绿（新增 3 项：models.json 同名不同 api_base 归并为一块、import_into_config 同名导入只写一个块（含不同端点分支）、opencode 同名 `entry.name` 归并；另加 chat.rs 重复 Provider 名可加载测试，改写原 `-2` 后缀测试）

---

## v0.6.11（2026-10-19）

### 新增
- **`ai config --import` 支持 opencode**：`parse_opencode_config` 解析 `opencode.json` 的 `provider.<id>` 块（`options.baseURL` / `options.apiKey` / `models` 对象，`models.*.name` 优先；opencode 无 kind 字段 → 全部按 OpenAI 兼容导入）；缺 key / 无模型的块跳过并提示；与 zcoder 同样走「新增优先、同名保留本地」合并
- **`--src DIR` 指定 agent 配置目录**：`software_config_path_with` 把各格式约定文件名（codex `config.toml`、opencode `opencode.json`、litellm `config.yaml`、freellmapi `freellmapi.config.json`、codebuddy / workbuddy `models.json`、trae `trae_cli.yaml`、zcoder `config.json`，`config_file_name`）直接在 `DIR` 下查找，替代默认 home 相对路径；作用于 **`--show` / `--update` / `--import` / `--list`** 四种模式的缺省路径（`-f FILE` 显式路径始终优先）；agent 装在非默认位置时无需记完整文件名

### 变更
- 版本 0.6.10 -> 0.6.11；`AiConfigArgs` 增加 `--src`；`cmd_config` / `cmd_config_update` / `cmd_config_import` 增加 `src` 参数；README（中 / 英）`ai config` 段落 + 特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys` 355 -> 358 项全绿（新增 3 项：opencode 配置反向解析、`config_file_name` / `software_config_path_with` 的 `--src` 映射（8 格式）+ 无 `--src` 时仍走 home 约定、opencode 经 `--src` 目录端到端导入）

---

## v0.6.10（2026-10-19）

### 变更
- **`ai config --list` 只列出存在的配置文件**：从「列出全部 8 个格式 + 默认路径」改为只显示默认配置文件**实际存在**的格式（`config_existing_list` 按 `path.exists()` 过滤）；全部不存在时 stderr 提示 `no existing config files found`；freellmapi 的 cwd 相对路径同样按存在性过滤（当前目录没有 `freellmapi.config.json` 就不显示）
- 版本 0.6.9 -> 0.6.10；README（中 / 英）`ai config` 段落与特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys` 354 -> 355 项全绿（新增 1 项：`config_existing_list` 只返回真实存在的路径、且是完整列表按同序的子集）

---

## v0.6.9（2026-10-19）

### 新增
- **`ai config --import` 反向导入**（`sys ai config <format> --import [-f 源头] [-c 目标]`）：从第三方工具配置文件读取 Provider / 模型参数，合并写回本地配置 `~/.sysenv/config.yaml`（写入前自动备份 `.bak-*`）。先支持 **codebuddy / workbuddy / zcoder** 三种格式（其余格式报 `--import is not supported`）；`-f FILE` 指定源头文件（缺省该格式默认路径，源文件不存在报错），`-c` 指定 sysenv 目标配置
- **models.json 反向解析**（codebuddy / workbuddy 共用，数组根与 `{"models":[...]}` 对象根均可）：`id` → 模型名；`url` 去掉尾部 `/chat/completions` 还原 `api_base`；`apiKey` → `api_key`；`maxInputTokens` / `maxOutputTokens` / `supportsImages` 映射 `max_input_tokens` / `max_tokens` / `type`；`vendor` 有真实名（非空且非 `Custom`）作 Provider 名，否则取 `api_base` 主机名（真实 workbuddy 手加条目 vendor 恒为 `Custom`）；同 (Provider 名, api_base) 归组，同名不同 api_base 的组名追加 `-2` 等后缀
- **ZCode config.json 反向解析**（zcoder）：`provider.<id>` 块 → Provider（`name` 优先取块的 `name` 字段），`options.baseURL` / `options.apiKey` / `models`（对象键 → 模型名，`models.*.name` 优先）映射到 sysenv 字段，`kind: anthropic` → `type: anthropic`
- **合并策略：新增优先，同名保留本地**（行级插入，未触及行保持原字节）：本地没有的 Provider 追加完整块（2/4/6 缩进，插在 `clients` 列表末尾 / 其它顶层键之前；`clients:` 缺失时创建）；已有 Provider 只追加本地缺失的模型条目（插在模型区之后，含子字段）；同名模型保留本地并报告 `skipped`；缺 `id` / `url` / `apiKey`（或 zcoder 缺 key / 无模型）的条目跳过并提示；全部已存在时提示 `nothing to import` 且不备份不写文件
- 报告：`backed up ...` / `added N model(s) in M provider(s): ...` / `skipped (already present): ...` / `skipped N incomplete entry/entries: ...`

### 变更
- 版本 0.6.8 -> 0.6.9；`AiConfigArgs` 增加 `--import`（与 `--show` / `--update` / `--list` 互斥）；README（中 / 英）`ai config` 段落与特性行、CHANGELOG（中 / 英）、requirements.md 同步
- `cargo test --bin sys` 345 -> 354 项全绿（新增 9 项：workbuddy 数组根解析、codebuddy 对象根解析、zcoder 解析、同名不同 api_base 去重后缀、新 Provider 块插入位置与备份、已有 Provider 追加模型 / 同名 skipped / 幂等不重写、`clients:` 缺失创建、无 `models:` 的 Provider 补建列表、不支持格式拒绝）

---

## v0.6.8（2026-10-19）

### 新增
- **`file --enc` 编码转换**：`sys file --enc SRC [DST] [--from ENC] [--to ENC]` 在 `utf-8` / `gbk` / `gb2312` 之间转换文本文件（**默认目标 utf-8**）；`--from` 缺省自动检测源编码（与 `data` 同一套启发：UTF-8 严格 + GBK 清洗对照，检测不出报错提示 `--from`）；DST 缺省输出到 stdout，**DST 必须不同于 SRC**（拒绝原地覆盖）；UTF-8 BOM 剥离、输出不带 BOM；`gb2312` 按 GBK 编解码（GB2312 是 GBK 子集）
- 标记从 ARGS 扫描（与 `-md5`/`-sha`/`-l` 同机制）：`--enc` 及别名 `--encode` / `--convert`，`-f ENC` 等价 `--from ENC`；因 `-e`（ext）是已注册短旗标，单杠 `-enc` 会被 clap 吃掉，需用双杠 `--enc`（已写入帮助/README）；`--enc` 与 `-md5`/`-sha` 同用报错

### 变更
- 版本 0.6.7 -> 0.6.8；README（中 / 英）`file` 特性行与命令参考同步 `--enc`；file.rs 模块头、main.rs FileArgs 帮助文本同步；dasel.rs `decode_clean` / `has_cjk` 改 `pub(crate)` 供 file 复用
- `cargo test --bin sys` 341 -> 345 项全绿（新增 4 项：`--enc` 标记与 `--from`/`--to`/别名扫描、编码名别名与未知报错、源编码自动检测（UTF-8 / GBK / 陷阱 / 检测失败）、转换端到端回环（显式与自动检测、gb2312 别名、BOM 剥离、同路径拒绝、源文件不被破坏））

---

## v0.6.7（2026-10-19）

### 新增
- **模型分组 `group`**：config.yaml 的 `clients` 下每个模型条目可加 `group` 参数，**支持多分组**，半角 / 全角逗号分隔（如 `group: fast, cheap`）；未配置的模型分组为空
- **分组选择器 `group:<名>`**：顶层 `model` 与 `-m/--model`（chat / image / config 导出同规则）可用分组名选中**所有归属该分组的模型**（跨 Provider、大小写不敏感、按 `weight` 加权轮询、重叠去重）；未命中报错并列出可用分组（`no model belongs to group \`x\` (available groups: ...)`），完全未配置分组时给出配置提示
- **分组展示**：`ai chat --list-model` 模型行显示 `[group: ...]`；`ai info model` 文本显示 `(group: ...)`、md 显示 `- group：...` 行、JSON/YAML 增加 `groups` 字段、CSV 增加 `groups` 列（多分组用 `|` 分隔）

### 变更
- 版本 0.6.6 -> 0.6.7；README（中 / 英）`clients` 模型字段、`model` / `-m` 选择器规则、`info model` 输出、`ai image` 选择规则同步 `group`
- `cargo test --bin sys` 339 -> 341 项全绿（新增 2 项：`group` 字段解析（多分组 / 空串 / 缺省）、`group:<名>` 选择器（跨 provider / 大小写 / 逗号混合去重 / 未命中报错 / 空名 / 无分组提示））

---

## v0.6.6（2026-10-19）

### 修复
- **`ai config workbuddy --update` 不再报 `not a JSON object`**：workbuddy 真实 `models.json` 顶层是**数组**（`[{id,name,vendor,url,apiKey,...}]`），merge 现同时接受数组根与 codebuddy 对象根（`{"models":[...],"availableModels":[...]}`）；数组根直接把数组当模型列表去重合并、写回仍为数组；非对象非数组才报错

### 新增
- **`ai config zcoder`（ZCode 格式）**：导出 / `--update` 目标 `~/.zcode/cli/config.json`（Windows `%USERPROFILE%\.zcode\cli\config.json`）；每个 provider 一个 `provider.<id>` 块（id=provider 名小写化、非法字符转 `-`），含 `kind`（anthropic / openai-compatible）、`options.baseURL` + `apiKey`、`models`（按模型 id 键控的对象）；`model.main` / `model.lite` 指向所选模型（`provider/model`）；`--update` 保留已有 provider 块与其它顶层键、`main/lite` 仅缺省时填入
- **`ai config workbuddy` 导出改为纯数组**（匹配真实 workbuddy schema；codebuddy 保持对象形状不变）
- **`ai config --list`**：列出全部 8 个支持格式（codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder）与其默认配置文件路径（`--list` 时可省略 FORMAT 参数）

### 变更
- 版本 0.6.5 -> 0.6.6；README（中 / 英）`ai config` 小节同步 workbuddy 数组形状 / zcoder / `--list`；路径映射新增 zcoder / qoder（qoder 格式按用户拍板本次未实现）
- `cargo test --bin sys` 333 -> 339 项全绿（新增 6 项：workbuddy 数组导出与数组根 merge、zcoder 导出 / merge / 缺省 main 填充、`--list` 全格式覆盖）

---

## v0.6.5（2026-10-19）

### 新增
- **JSONL（JSON Lines / NDJSON）格式**：`-i jsonl` / `-o jsonl` / `.jsonl` / `.ndjson` 后缀；输入每行一个 JSON 值解析为**根数组**——选择器、`[*].filter(...)`、`search(...)`、`--limit` 全部按 JSON 数组工作；**裸谓词自动过滤**与 csv / table 一致（`sys data -f app.log 'level=="error"'` 直接可用）；输出为逐行紧凑 JSON；内容检测兜底（整文非单一 JSON 但每行都是 JSON 时判为 jsonl，`.log` 日志文件无需 `-i`）
- 错误行报错带行号（`invalid JSONL at line N`）

### 变更
- 版本 0.6.4 -> 0.6.5；README（中 / 英）data 特性行更新为 JSON/JSONL/YAML/TOML/CSV/MD/TABLE 七格式；模块头注释补充 jsonl 示例
- `cargo test --bin sys` 329 -> 333 项全绿（新增 4 项：解析+回环、行号报错、格式检测区分、裸谓词过滤+limit 组合）

---

## v0.6.4（2026-10-19）

### 修复
- **非 UTF-8 输入不再报错**：`sys data` 读取文件 / stdin 改为字节级解码，按 UTF-8（BOM 剥离声明）→ UTF-16（BOM）→ GBK（Windows ANSI 936）→ GB18030 → 宽松降级（U+FFFD）的顺序解码；并处理 GBK 字节对恰好是合法 UTF-8 的摩斯码陷阱（UTF-8 视图无汉字而 GBK 视图含汉字时取 GBK）——`sys data -f xxx.log`（GBK 设备日志）不再报 `stream did not contain valid UTF-8`
- **格式检测失败不再 panic**：单行输入且无 `-i` 时 `detect_format` 报错改为正常错误返回（原 `.expect()` 直接 panic），提示 `cannot detect the input format`
- 新增依赖 `encoding_rs`（纯 Rust，跨平台）

### 变更
- 版本 0.6.3 -> 0.6.4；README（中 / 英）data 特性行补充解码说明
- `cargo test --bin sys` 326 -> 329 项全绿（新增解码 3 项：UTF-8/BOM、GBK-恰似-UTF-8、GBK 非法 UTF-8 + UTF-16 + 宽松降级）

---

## v0.6.3（2026-10-19）

### 新增
- **`--limit N`**：`sys data` 新增行数截断——文档根为数组时仅保留前 N 行（`-i csv` / `-i table` 解析结果为行对象数组，即只看前 N 条数据、表头仍输出）；在表达式前应用（截断后的文档上执行查询 / 过滤）；`N=0` 无输出（head -n 0 语义）；非数组根明确报错

### 变更
- 版本 0.6.2 -> 0.6.3；README（中 / 英）data 特性行补充 `--limit`；模块头注释同步
- `cargo test --bin sys` 323 -> 326 项全绿（新增 apply_limit 截断 / 非数组报错 / table+csv 全链路 3 项）

---

## v0.6.2（2026-10-19）

### 新增
- **csv / table 裸比较表达式自动 filter**：`-i csv` / `-i table` 下含比较运算符（`== != > >= < <=`，引号外）或 `!` 的表达式自动当作根数组 filter——`ps | sys data -i table 'Id==7360'` 直接可用（等价 `[*].filter(Id==7360)`）；仅 csv / table 触发，其余格式与赋值 / 函数表达式行为不变

### 变更
- 版本 0.6.1 -> 0.6.2；README（中 / 英）data 特性行补充裸谓词示例
- `cargo test --bin sys` 321 -> 323 项全绿

---

## v0.6.1（2026-10-19）

### 变更
- **csv / table 表头判定规则修正**：第一行默认即列名；第二行为 `----` 分隔线时识别为表头下划线并跳过（ps 输出的 `-------` 行不再成为垃圾数据）；`--no-header` 是启用 `col1..colN` 的唯一途径（不再自动推断），`--header` 强制首行为列名
- 版本 0.6.0 -> 0.6.1；README（中 / 英）data 特性行同步
- `cargo test --bin sys` 319 -> 321 项全绿

---

## v0.6.0（2026-10-19）

### 变更
- **CSV / table 列标题统一虚拟为对象键**：`-i csv` 表头语义对齐 table——首行含非数字单元格即作表头（列标题为键名），全数字首行视为数据、自动生成 `col1..colN`；`--header` / `--no-header` 对 `-i csv` 同样生效；空表头单元格回退 `colN`
- 版本 0.5.13 -> 0.6.0；README（中 / 英）data 特性行更新为 CSV/TABLE 表头统一说明
- `cargo test --bin sys` 315 -> 319 项全绿

---

## v0.5.13（2026-10-19）

### 新增
- **根数组函数表达式**：裸 `[*]` 在根为数组时寻址数组本身，`sys data -i table '[*].filter(Id==7360)'` / `'[*].search(...)'` 可直接过滤 / 搜索整张表（filter / search 共用 `resolve_single`，一并生效）

### 变更
- 版本 0.5.12 -> 0.5.13；README（中 / 英）data 特性行补充根数组 `[*].filter(...)`
- `cargo test --bin sys` 314 -> 315 项全绿

---

## v0.5.12（2026-10-19）

### 新增
- **`sys data -i table`**：解析空格分隔的表格输出（如 `ps`），每行一条数据；首非空行含非数字单元格即作列标题（可 `[0].PID` / `[*].COMMAND` 按列名查询），否则生成 `col1..colN`；`--header` / `--no-header` 强制指定（互斥）；多余列并入最后一列、缺列补空、类型推断同 CSV
- **`-o table`**：对象数组 / 单对象 / 纯标量数组对齐输出；标量读取（如 `[0].PID`）直接打印数字 / 布尔 / 字符串
- **根数组选择器**：`[0]` / `[-1]` / `[*]` 可作为选择器开头寻址根数组

### 变更
- 版本 0.5.11 -> 0.5.12；README（中 / 英）data 特性行更新为六格式互转 + table 解析
- `cargo test --bin sys` 300 -> 314 项全绿

---

## v0.5.11（2026-10-17）

### 移除
- **`sys search` 子命令整体删除**（联网搜索，博查 Bocha AI Web/AI Search API）。删除 `Cmd::Search` / `SearchArgs` / 分发与帮助文本；chat.rs 移除 `Config.search` 与 `SearchKey`、配置 `search:` 段解析、Bocha 全部代码（10 个 bocha_* 函数 + 2 个端点常量）、`cmd_search` 与 8 项测试；short.rs 移除 `ssearch` 垫片（8 → 7）
- 本机已安装的 `ssearch.cmd` 不主动清理（发布约定不更新 short），重跑 `sys short` 后消失

### 变更
- 版本 0.5.10 -> 0.5.11；README（中 / 英）删除 search 特性行、`5.6/5.3 网络搜索` 章节与垫片表格 ssearch 行（后续章节重编号；垫片说明更新为七种 + 示例数字 6 → 7）
- `cargo test --bin sys` 308 -> 300 项全绿

---

## v0.5.10（2026-10-17）

### 移除
- **`sys con` 子命令整体删除**：格式转换由 `sys data`（`dasel`）完全覆盖（JSON / YAML / TOML / CSV / MD，`-i` / `-o` / `-f` / `-w`）。删除 `Cmd::Con` / `ConArgs` / 分发与帮助文本；convert.rs 移除 con 专属代码（`Format` / `detect_format` / `parse` / `parse_yaml_value` / `parse_args` / `CON_HELP` / `cmd_con`）与 4 项测试，**保留** dasel 共享解析 / 序列化函数并改写模块文档；short.rs 移除 `scon` 垫片（9 → 8）
- 本机已安装的 `scon.cmd` 不主动清理（发布约定不更新 short），重跑 `sys short` 后消失

### 变更
- 版本 0.5.9 -> 0.5.10；README（中 / 英）删除 con 特性行、`5.7/5.8 格式转换` 章节与垫片表格 scon 行（垫片说明更新为八种 + data/log 无垫片）
- `cargo test --bin sys` 312 -> 308 项全绿

---

## v0.5.9（2026-10-17）

### 新增
- **`sys log`（彩色 less 式日志查看器）**：解析常见日志并高亮着色展示，交互同 less（PageUp / PageDown 翻页、↑/↓ 逐行、Home / End / g / G 跳转、q / Esc / Ctrl-C 退出）
  - **时间解析（多种常见格式，均识别为日志开头并以橙色 RGB(255,165,0) 显示）**：ISO `2024-01-15 10:23:45`、`2024-01-15T10:23:45.123Z`、Log4j 逗号毫秒 `2024-01-15 10:23:45,123`；斜杠 `2024/01/15 10:23:46`；nginx access log `15/Jan/2024:10:23:49 +0800`（时间位于行中间也能识别，行内前 80 字节扫描，前缀 `127.0.0.1 - - [` 完整保留）；syslog `Jan 15 10:23:48`（无年份取当年，落在未来 1 天以上自动回退上年）；US `01/15/2024`
  - **级别渐变色**：debug / info / warning / error 等识别后按级别着色——trace 浅绿(178,230,178)、debug 绿(102,204,102)、info 绿(0,180,0)（info 及以下为绿色、级别越低越浅），warning 红(255,90,90)、error 深红(204,0,0)、fatal 最深红(140,0,0) 加粗
  - **app 标志**：时间后形如 `[http-nio-8080-exec-1]`、`app[1234]`、`[01-node-5]` 的应用标记以深黄色 RGB(184,134,11) 显示
  - **`--file FILE`**：指定日志文件（缺省读 stdin，`-` 亦为 stdin）
  - **`--level LEVEL`**：按级别过滤（trace | debug | info | warn | warning | error | fatal；warn==warning、err==error、critical/crit/alert/emerg==fatal；无级别行剔除）
  - **`--search KEYWORD`**：搜索关键字（大小写不敏感）——显示行号（灰色），命中反显高亮；交互下 **n 跳下一条匹配 / u 跳上一条匹配**（循环，命中行居中）
  - **`--line N`**：从第 N 行之后开始解析（保留原始行号）
  - **`--newer TIME`**：只显示时间在指定时间之后的日志（无时间戳的延续行保留）
  - **`-f`**：跟踪文件变化——150ms 轮询增量读取、文件截断 / 轮转自动重读、滚动展示；仅文件可跟随（stdin 报错退出码 1）
  - 非 TTY（管道 / 重定向）输出纯文本无 ANSI；TTY 进 crossterm 全屏交互，底部状态栏显示文件名 / 行范围 / 级别过滤 / 搜索词与命中计数 / 跟随状态与按键提示
  - 新依赖 crossterm 0.28；新增 29 项 `log::tests` 单测；`cargo test --bin sys` 312 项全绿

### 变更
- 版本 0.5.8 -> 0.5.9；README（中 / 英）新增 `sys log` 章节与示例
- `src/convert.rs`：`parse` / `yval_to_json` / `parse_csv_rows` / `cell_to_value` / `parse_md_row` 提升为 `pub(crate)`（v0.5.8 dasel.rs 编译所需，随本次提交补齐）

### 冒烟验证（真实二进制，Windows）
- `sys log --file dev/sample.log`：纯文本 8 行完全复原（含 nginx 行内时间前缀）；`--level warn` 只留 WARN 行；`--search app` 8 行带行号；`--newer "2024-01-15 10:23:47"` 过滤正确；`--line 6 --search GET` 正确；非法 `--level bogus` / `--newer garbage` 报友好错误退出码 1；stdin 管道 `-f` 报错退出码 1
- 真实控制台 TTY 交互冒烟：查看器启动进入 raw mode / 全屏渲染不崩溃（进程存活等待按键）

---

## v0.5.8（2026-10-17）

### 新增
- **`sys data` 函数表达式（dasel 兼容）**：`list.filter(谓词)` 过滤数组元素；`list.remove(谓词)` 删除匹配元素（原地修改，命中的数组元素即"包含此内容"的父节点）；`search(谓词)` 递归收集匹配节点；字符串 / 键函数 `name.endsWith("x")`（contains / startsWith / endsWith / indexOf / has）
- **谓词语言**：`$this` / `$key`（当前节点 / 键或下标）、字段路径（`name`、`a.b[0]`、`m["中文键"]`）、字面量（数字 / 引号字符串 / true / false / null）、比较与逻辑（`== != > >= < <=`、`&&`、`||`、`!`）、函数调用（`contains("x")` / `startsWith("x")` / `endsWith("x")` / `indexOf("x")` / `has("key")` / `has(0)`）；字符串字面量支持单引号（PowerShell 传参兼容）
- **`sys data --depth N`**：删除层级大于 N 的节点（根为第 0 层，0..=N 层保留，更深子树整棵移除）；裸 `--depth` 即 1；在表达式求值前应用
- **赋值识别修正**：`split_expression` 跳过 `==` / `!=` / `>=` / `<=` 比较符中的 `=`，函数表达式不再被误拆为赋值
- 新增 7 项单测（函数表达式解析 / 谓词求值 / filter / remove / search / 字符串函数 / depth 修剪）；`cargo test` 283 项全绿

### 移除
- **`sys ai task` 模块整体删除**：chat.rs 的 Task 结构、`tasks` 配置解析、全部 task 工具（task_tools / route_tasks / 内置搜索源 / 打分匹配）与相关测试移除；ai.rs / main.rs 的 task 字段、`AiCmd::Task` 变体与分发分支移除；`sys ai --help` 不再显示 task

### 变更
- README（中 / 英）删除 `ai task` 章节与特性行，补齐 data 函数表达式 / `--depth` 文档；版本 0.5.7 -> 0.5.8

---

## v0.5.7（2026-10-17）

### 新增
- **`sys data`（别名 `dasel`）**：结构化数据查询 / 修改 / 转换（JSON / YAML / TOML / CSV / MD）。dasel 兼容选择器（`a.b[0]` / `list[-1]` / `list[*]` / `m["中文名"]`）；表达式修改（`PATH = VALUE`、`= null` 删除、`+=/-=/*=/=/` 运算）；`-f FILE`（后缀定格式）/ `-u URL`（GET，默认 json，`-i` 覆盖）/ stdin；`-i/-o` 格式指定、`-w` 写回、`--root`、`--compact`
- **tab 缩进 YAML 自动归一**：前导 tab 转 4 空格（引号内 tab 保留）后严格解析，失败回退容错块解析器；格式探测 YAML 前置结构启发式，CSV/MD 优先，避免纯文本误判
- **中文兼容验证**：JSON↔YAML↔TOML↔CSV 往返 CJK 不转义不乱码；多行中文块标量往返一致；新增 16 项单测
- 新依赖：toml 0.9、libc 0.2

### 修复
- 方括号内引号键 `m["中文名.字"]` 被误当作数组下标
- 无结构逗号文本被 detect_format 误判为 YAML

---

## v0.5.1（2026-10-13）

### 变更
- **`sys path add` 默认仅当前会话（`--save` 才持久化）**：不加 `--save` 时只更新当前进程并打印一条可粘贴片段，不写入任何持久化存储；`--save` 持久化（user/machine 作用域）供新进程继承；原 `--temporary` 标志移除（与默认行为重复）
- **修复临时模式片段打印两遍**：原实现临时分支与持久化分支各打印两遍可粘贴片段；现统一收敛为单条片段
- README.md / README.en.md 同步；版本 0.5.0 -> 0.5.1；`cargo test` 257 项全绿（冒烟验证：默认 add 不持久化、`--save` 持久化且可 `path remove` 清理、旧 `--temporary` 报错）

---

## v0.5.0（2026-10-12）

### 变更
- **`sys ai config` 缺省导出模型 = `sys ai chat` 的默认模型**：不加 `-m`（也不给位置参数 `SELECT`）时，不再导出全部模型，而是与 `sys ai chat` 一致——解析配置顶层 `model:` 选择器（含逗号多选），顶层未配置时取第一个 Provider 的第一个模型；`-m` / 位置参数显式给出时行为不变
- 版本 0.4.72 -> 0.5.0；`cargo test` 256 -> 257 项全绿

---

## v0.4.72（2026-10-12）

### 新增
- **`sys ai config` 新增 codebuddy / workbuddy / trae 三种导出格式**：
  - **`codebuddy`**（`~/.codebuddy/models.json`）与 **`workbuddy`**（`~/.workbuddy/models.json`）：共用 models.json schema，每个模型一条 `id` / `name` / `vendor` / `url`（`{api_base}/chat/completions`）/ `apiKey`（内联）；配置里有 `max_input_tokens` / `max_tokens` / `type`（含 `image`）时带上 `maxInputTokens` / `maxOutputTokens` / `supportsImages`；同名模型 id 去重（首个 Provider 保留），`availableModels` 同步
  - **`trae`**（trae_cli.yaml）：YAML `models` 列表，OpenAI 兼容 Provider 用 `open_ai` 块、anthropic 用 `claude` 块，按模型 id 去重；默认路径 Windows `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`，Linux `$XDG_CONFIG_HOME`（未设则 `~/.config/trae_cli/trae_cli.yaml`）
- **`-m/--models` 多模型选择**：与 `sys ai chat -m` 完全一致的格式（`provider:model` / `provider:*` / 裸模型名，半角/全角逗号分隔可指定多个，大小写不敏感，重叠选择去重），复用 `chat::resolve_targets`；原位置参数 `SELECT` 保留为旧式单选择器（与 `-m` 互斥）
- **`--update` 就地合并更新**：按指定格式到该软件默认配置目录找文件（`-f FILE` 可覆盖路径），文件不存在时提示 `config file not found ... nothing to update`；存在则先备份 `.bak-YYYYMMDD-HHMMSS`（不覆盖旧备份），再合并写入缺失的模型条目（保留已有条目与其它顶层键），成功后提示 `added N model(s): ...`（无新增时提示 `nothing to add`）；仅 codebuddy / workbuddy / trae 支持，其它格式明确报错；与 `--show` 互斥
- README / CHANGELOG 同步；版本 0.4.71 -> 0.4.72

### 修复
- **合并解析前剥离 UTF-8 BOM**：Windows 编辑器保存的 models.json / trae_cli.yaml 常带 BOM，严格 JSON / YAML 解析会失败（`expected value at line 1 column 1`）；读入后先 `strip_prefix('\u{feff}')` 再解析
- 新增 9 个单测（chat 格式多选/去重、codebuddy/trae 导出结构与去重、models.json 合并与二次合并 no-op、trae YAML 合并保留其它键、缺失文件提示、不支持格式报错、BOM 容忍、默认路径映射）；`cargo test` 247 -> 256 项全绿

---

## v0.4.71（2026-10-12）

### 新增
- **`sys ai model --src openrouter`**：新增 `--src` 参数（默认 `models`，不提供时行为与旧版完全一致）；`--src openrouter` 时 HTTP GET 访问 `https://openrouter.ai/api/v1/models`，列表显示模型的 `id` / `name` / `context_length` / 价格（取 `pricing.prompt`，每 1M token）/ 创建时间（`created` unix 时间戳转为本地时间）；支持模型名称过滤（如 `sys ai model deepseek --src openrouter`），不提供时全列表；支持 `--limit` / `--json` / `-o json|csv|yaml|md`
- **`sys ai price` 独立命令（完全接管 `info price`）**：`sys ai price P1,P2,...` 查 models.dev 价格；新增 `--src` 参数（默认 `models`）；`--src openrouter` 时展示 openrouter.ai 每个模型 `pricing` 的 `prompt` 与 `completion` 价格（每 1M token，USD），可选位置参数作模型名称过滤；支持 `--limit` / `--json` / `-o json|csv|yaml|md`
- **删除 `sys ai info price`**：功能与 `sys ai price` 重复
- README.md / README.en.md 同步用法；版本 0.4.70 -> 0.4.71

---

## v0.4.39（2026-10-09）

### 变更
- **`sys ai chat --update`：显式补齐缺失的能力字段并写回配置**。不传该参数时保持 v0.4.38 的解耦行为（零查询、零写回）；传了才在发送前把本次目标模型**缺失**的 `max_input_tokens` / `type` 从 models.dev（24h 缓存）补齐并持久化到配置文件，best effort（查询或写入失败仅警告、不阻塞请求），字段已齐的模型不查询；stderr 打印实际补齐内容（如 `sys: update: filled max_input_tokens=..., type=... for ...`）
- 恢复 ai.rs `lookup_model_capabilities`（exact 命中优先、fuzzy 次之，失败返回空）供 `--update` 使用；`Model.type` 字段随解析恢复（`--update` 需要判断 `type` 是否缺失；配置中已有的 `type: text,image` 行重新生效）
- 抽出可单测的 `missing_capability_fields`（判定哪些字段缺失），新增单元测试；chat 的 `cmd_task` 路径不接 `--update`，保持零查询
- 示例配置 `config.example.yaml`、README 5.2 与功能总览表同步更新
- 版本 0.4.38 -> 0.4.39

## v0.4.38（2026-10-09）

### 变更
- **`sys ai chat` / `sys ai task` 彻底解耦 models.dev**：移除发送请求前的「自动补齐能力字段」逻辑（`ensure_model_capabilities`），模型缺 `max_input_tokens` / `type` 时**不再查询 models.dev（联网或 24h 缓存）也不再写回配置**；能力字段完全只读配置文件——缺 `max_input_tokens` 即不截断消息，`type` 缺失不影响请求
- 删除 ai.rs 中因此不再被使用的 `lookup_model_capabilities`；`Model.type`（模型级模态字段）无任何消费者，一并移除（配置里已有的 `type: text,image` 行仍可保留，解析时自动忽略）
- 示例配置 `config.example.yaml` 与 README 5.2 同步更新说明
- 版本 0.4.37 -> 0.4.38

## v0.4.37（2026-10-09）

### 变更
- **移除 `sys env config` 子命令**（v0.4.36 新增，超出请求范围）：`ai config` 的 `--show` / 备份 / 报告能力只保留在 `sys ai config` 入口，`sys env` 恢复为 get / set / unset / list
- **`sys ai config` 定稿**：
  - `--show` 预览：先打印对应软件的配置文件地址，再打印将写入的配置（不触碰任何文件）；同时给了 `-f FILE` 时额外打印 `write target` 行
  - `-f FILE` 写入报告改为打印 **provider 数 + 模型数 + 目标文件路径**（原先只有模型数），备份信息不变
  - `backup_file` 同名备份冲突时自动追加 `-N` 后缀，同一秒内多次备份不会互相覆盖
  - 新增 `software_config_path_in` / `backup_file` 的确定性单元测试（格式映射、备份命名与内容保全）
- 版本 0.4.36 -> 0.4.37

## v0.4.36（2026-10-09）

### 新增
- **`sys env config` 子命令**：与 `sys ai config` 同参数、同实现，用于查看 / 应用软件配置导出（`codex` | `opencode` | `litellm` | `freellmapi`）：
  - **`--show`**：先打印对应软件的配置文件地址（codex → `~/.codex/config.toml`、opencode → `~/.config/opencode/opencode.json`、litellm → `~/.litellm/config.yaml`、freellmapi → 当前目录 `freellmapi.config.json`，无默认路径时提示用 `-f` 指定），再打印将要写入该软件的配置信息，不触碰任何文件
  - **`-f FILE` 实际修改**：写入前先把已有配置文件备份为 `FILE.bak-YYYYMMDD-HHMMSS`（同目录），写入后打印实际修改信息——导出的模型数 / 目标文件路径 / 备份文件路径；目标文件原本不存在时打印“新建配置文件”
  - `sys ai config` 同步新增 `--show` 与备份逻辑，两入口行为一致
- 版本 0.4.35 -> 0.4.36

## v0.4.35（2026-10-09）

### 变更
- **`sys ai info list-model` 文本视图的 `CREATED` 时间戳转为本地日期时间**：openai 兼容 provider 返回的 unix 秒级时间戳在文本视图自动显示为本地时间 `YYYY-MM-DD HH:MM:SS`（基于系统时区）；anthropic 的 ISO `created_at` 及空/非数字值原样透传；`-o json` / `-o csv` 保持 API 原始时间戳不变（机器可读）。新增 `fmt_created` 纯函数并附带往返校验单元测试
- 版本 0.4.34 -> 0.4.35

## v0.4.34（2026-10-09）

### 新增
- **`sys ai info` 新增 `list-model PROVIDER`**：用该 provider 自己的 models 接口实时查询其可用模型列表——OpenAI 兼容 provider 请求 `GET {api_base}/models`（Bearer api_key），anthropic 类型请求 `GET {api_base}/v1/models`（api_base 已以 `/v1` 结尾时直接用 `{api_base}/models`，x-api-key / anthropic-version）；Anthropic `next_page` 分页自动翻页（上限 20 页）。文本视图打印 `MODEL / OWNED_BY / CREATED` 对齐表（openai 的 `owned_by` / `created`、anthropic 的 `display_name` / `created_at` 自动归一为同列）；`-o json` / `-o csv` / `--json` 输出 JSON 数组或 CSV 表（字段 `id` / `owned_by` / `created`）。实时查询，不走缓存
- 版本 0.4.33 -> 0.4.34

## v0.4.33（2026-10-08）

### 新增
- **`sys ai model` 新增 `--price PRICE`**：只保留 `cost.input` 与 `cost.output` 均 <= PRICE 的模型（每 1M tokens 美元，缺失 cost 字段按 0 计）；参数可选，未提供时默认值为 0，即只显示免费/未标价模型；与 `--provider` / `--model-type` / `--date` / `--open` 及名称搜索叠加生效，文本视图页脚与无匹配提示标注价格条件
- 版本 0.4.32 -> 0.4.33

## v0.4.32（2026-10-08）

### 新增
- **`sys ai model` 新增 `--provider PROVIDER`**：只列出 provider id 或名称包含该值的模型（大小写不敏感子串匹配，如 `--provider openai` / `--provider 阿里`），与 `--model-type` / `--date` / `--open` 等过滤叠加生效；无参列表与 `NAME` / `--search` 查询均适用，文本视图页脚与无匹配提示会标注 provider 过滤条件
- 版本 0.4.31 -> 0.4.32

## v0.4.31（2026-10-07）

### 新增
- **`sys file` / `sys con` 快捷垫片 `sfile` / `scon`**：`sys short` 现在安装九个短命令（spath/senv/slink/shttp/sai/ssearch/stask/sfile/scon），`sfile` 等价 `sys file`（文本搜索/替换/文件查看），`scon` 等价 `sys con`（json/csv/md/yaml 转换）
- 版本 0.4.30 -> 0.4.31

## v0.4.30（2026-10-07）

### 新增
- **`sys ai info provider` 支持 `-o yaml` 并作为默认格式**：无 `-o` 时默认输出 YAML 列表（每个 provider 一个 `- name:` 条目，含 api_base / api_key / docs / console），`-o yaml` 显式等价；`-o json` / `--json`、`-o csv` 行为不变；`ai model` / `ai provider` / `ai cn-model` / `ai info`（无参与 model）等其余命令的 `-o` 同步支持 yaml 输出
- 版本 0.4.29 -> 0.4.30

## v0.4.29（2026-10-07）

### 变更
- **`sys ai server --help` 分组信息精简**：provider 行只保留 name / api_base（去掉 api_key / docs / console），其后仍缩进列出该 provider 的全部模型 `{provider}:{name}`
- 版本 0.4.28 -> 0.4.29

## v0.4.28（2026-10-07）

### 变更
- **`sys ai server --help` 配置信息改为按 provider 分组显示**：每个 provider 一行（name / api_base / api_key / docs / console），其后缩进列出该 provider 的全部模型 `{provider}:{name}`（取代原先的 provider 表 + 独立模型清单两段式）
- 版本 0.4.27 -> 0.4.28

## v0.4.27（2026-10-07）

### 变更
- **`sys ai chat --server` 改为 `sys ai server` 子命令**：启动方式由 `sys ai chat --server [ADDR]` 迁移为 `sys ai server [ADDR]`（默认 `127.0.0.1:10000`，只给端口/IP 各保留另一默认，新增 `-c/--config` 指定配置文件）；`sys ai chat --server --help` 迁移为 `sys ai server --help`（探测服务是否已启动：已启动打印地址/端口，未启动提示 `server is NOT running`，两种情况均打印配置的 provider 表与模型清单）；`chat` 子命令移除 `--server` / 自定义 `--help`，恢复 clap 默认帮助
- 版本 0.4.26 -> 0.4.27

## v0.4.26（2026-10-07）

### 新增
- **`sys ai chat --server --help` 服务状态查询**：先探测该地址（GET /v1/models，2 秒超时；`0.0.0.0` / `::` 通配地址经 127.0.0.1 探测）确认服务是否已启动——已启动打印当前地址/端口，未启动提示 `server is NOT running` 并给出启动命令；两种情况都会继续打印当前配置的 provider 表（name/api_base/api_key/DOCS/CONSOLE）与全部模型清单（`{provider}:{name}`）。chat 子命令启用自定义 `-h/--help`（不带 `--server` 时显示用法摘要）
- 版本 0.4.25 -> 0.4.26

## v0.4.25（2026-10-07）

### 新增
- **`sys ai info` 无参默认输出**：不带 FIELD 参数时，同时打印当前配置的 provider 表（name / api_base / api_key / DOCS / CONSOLE）与全部模型清单（`{provider}:{name}`）；`-o json` 输出合并对象 `{"providers": [...], "models": [...]}`；无参 + `-o csv` 明确提示改用 `info provider` / `info model` 单独输出
- 版本 0.4.24 -> 0.4.25

## v0.4.24（2026-10-07）

### 新增
- **`sys ai config` 多格式配置导出**：把本地配置（`clients` 段）导出为 `codex`（TOML，`[model_providers]` 段 + `env_key` + stderr export 提示）、`opencode`（JSON，`@ai-sdk/openai-compatible` + 内联 apiKey）、`litellm`（YAML `model_list`，暴露名 `{provider}:{model}` 防冲突）、`freellmapi`（JSON `customProviders`）四种格式；缺省导出全部 provider 的全部模型，`provider:*` 导出指定 provider 全部模型、`provider:model` 导出单个模型（无匹配明确报错并列出可用项）；`-f/--file` 写入文件（缺省 stdout），`-c/--config` 指定配置；找不到配置文件沿用既有明确报错
- **`sys ai chat --server [ADDR]` OpenAI 兼容本地服务**：把配置中的 provider 以 OpenAI 格式对外提供，默认 `127.0.0.1:10000`；地址可只给端口（host 用默认）或只给 IP（端口用默认）；提供 `POST /v1/chat/completions`（非流式透传 + SSE 流式 chunked 转发）与 `GET /v1/models`（全部模型，ID 为 `{provider}:{model}`）；请求 model 支持 `provider:model` / `provider:*` / 裸名 / `auto`（= 配置默认）；仅服务 `type: openai` 的 provider，anthropic 目标明确报错；未知模型返回 404 并列出可用项
- 版本 0.4.23 -> 0.4.24

## v0.4.23（2026-10-07）

### 变更
- **项目全面改名 `sysenv` → `sys`**：可执行文件名（`sysenv.exe` → `sys.exe`）、命令输出（`sysenv: ...` 错误/警告前缀）、`--version`/`--help` 显示、User-Agent、垫片生成注释、文档命令示例与产物命名规范（`sysenv-windows-...` → `sys-windows-...`）全部改为 `sys`。
- **用户数据目录保留**：`~/.sysenv/config.yaml` 配置、`~/.sysenv/bin` 托管目录、`~/.sysenv/cache` 缓存路径均**不迁移**，改名后继续读写既有数据。
- **产物缓存目录已改**：Windows `%LOCALAPPDATA%\sysenv\` → `%LOCALAPPDATA%\sys\`，Linux `~/.cache/sysenv/` → `~/.cache/sys/`（缓存可再生，首次运行自动重新抓取）。
- 版本 0.4.22 -> 0.4.23

## v0.4.22（2026-10-07）

### 新增
- **`sys file` 单参数智能分派**：
  - 参数带扩展名（如 `me.txt`）→ **显示文件内容**（cat 风格，二进制安全显示；文件不存在时明确报错）
  - 参数被引号包裹（`"me.txt"` / `'me.txt'`）→ 剥掉引号按原 stdin 字符串搜索（引号作为"强制搜索"信号）
  - 参数无扩展名（如 `hello`）→ 保持原 stdin 搜索行为不变
  - 隐藏名（`.env`）与无点路径不算扩展名，仍按搜索处理；带 `-S`/`--newer`/`--older`/`-d` 筛选时仍走列文件模式
- 版本 0.4.21 -> 0.4.22

## v0.4.21（2026-10-07）

### 新增
- **`sys file` 文件搜索功能（fd 风格属性筛选）**：
  - `-S` / `--size SIZE`：只列/搜索大小达到指定值的文件。纯数字按字节；`2k`/`2m`/`2g`/`2t`（及 kb/mb/gb/tb）按 1024 进制换算，支持小数（`1.5m`），大小写不敏感
  - `--newer TIME`：只列/搜索修改时间不早于 TIME 的文件（mtime >= TIME）
  - `--older TIME`：只列/搜索修改时间早于 TIME 的文件（mtime < TIME）
  - `-d NUM`：只递归 NUM 级子目录（`-d 0` 仅当前目录）
  - 时间格式支持 `YYYY-MM-DD`、`YYYY-MM-DD HH:MM[:SS]`、`T` 分隔及斜杠分隔，按本地时区解释
  - 给出以上任一筛选且不带 PATTERN 时进入**列文件模式**（每行一个路径，默认当前目录；`-e` / `-t` 可附加筛选）；带 PATTERN / OLD NEW 时筛选叠加到字符串搜索与就地替换
  - 新增依赖 chrono 0.4（本地时区时间解析）
- 版本 0.4.20 -> 0.4.21

## v0.4.20（2026-10-07）

### 新增
- **`sys con`（json / csv / md / yaml 四种格式互转）**：
  - 默认从 stdin 管道读取（`cat a.json | sys con`）；`-file F` 改从文件读取（也可按扩展名推断输入格式）
  - `-i FMT` 指定输入格式：json | csv | md | yaml（省略时自动检测内容格式）
  - `-o FMT` 指定输出格式：json | csv | md | yaml（省略时按输入格式输出，即只格式化显示）
  - `-out F` 把结果写到文件，默认只输出到 stdout
  - 表格类转换（csv / md ↔ json / yaml）映射为对象数组：CSV 首行为表头，Markdown 表格转对象数组；单元格做 null / bool / int / float 类型推断；字段含逗号、引号、换行、`|` 时正确转义（`\|`）
  - YAML 解析先走 serde_yaml，失败时回退到项目内置 tab-tolerant 解析器（兼容真实配置中的 tab 缩进）
  - 新增依赖 serde_yaml 0.9；serde_json 开启 preserve_order 保持键序
- 版本 0.4.19 -> 0.4.20

## v0.4.19（2026-10-07）

### 新增
- **`sys file`（fd/sd 风格文本搜索与替换）**：
  - `file PATTERN`：从标准输入读取并输出匹配行（`type a.txt | sys file hello`）
  - `file PATTERN PATH`：在 PATH（文件或目录树）的已知文本与源码文件（txt/md/py/java/c/...）中搜索，输出 `路径:行号:内容`
  - `file OLD NEW PATH`：把 OLD 就地替换为 NEW，输出每个文件替换数与汇总
  - `-e EXT`（可多次，前导点可省略）扩展名过滤；`-i` 忽略大小写（搜索/替换均生效）；`-t` 仅纯文本（排除源码）；`-w` 整词匹配；`-c NUM` 上下文行（组间 `--`）
  - 匹配在 Unicode 字符层逐字比较（`-i` 对中文等非 ASCII 同样正确）；遍历跳过隐藏项与 `.git`/`node_modules`/`target` 等噪音目录，二进制文件自动跳过
- 版本 0.4.18 -> 0.4.19

## v0.4.18（2026-10-07）

### 新增
- **`sys ai info provider` 增加 DOCS / CONSOLE 列**：按 provider 名称匹配官方帮助文档与控制台地址（agnes / alibaba-cn / minimax / modelscope / anspire / sensenova / bigmodel / amd 已收录，URL 均已实测可达；未收录的 provider 显示 `-`）
  - 文本输出新增 `DOCS`、`CONSOLE` 两列（API_KEY 移至末列）
  - `-o json` 每条增加 `docs` / `console` 字段；`-o csv` 增加 `docs,console` 列
- 版本 0.4.17 -> 0.4.18

## v0.4.17（2026-10-07）

### 新增
- **`sys ai info provider` / `info model` 支持机器可读输出**：
  - `-o json` / `--output-format json`（或 `--json`）：输出 JSON 数组（provider 为 `{"name","api_base","api_key"}`，model 为 `{"provider","name"}`，`to_string_pretty` 格式化）
  - `-o csv` / `--output-format csv`：输出 CSV 表（provider：`name,api_base,api_key`；model：`provider,name`）
  - 与 `info provider [KEYWORD]` 过滤共用；`price` / `balance` / `sale-price` 传入 `-o/--json` 时明确报错提示
- 版本 0.4.16 -> 0.4.17

## v0.4.16（2026-10-07）

### 新增
- **`sys ai info` 支持阿里云百炼（`alibaba-cn`）**：
  - `info balance alibaba-cn`：调用官方 `GET /api/v1/models/limits` 验证 Key 并列出各模型用量限额（限流配额，非现金余额）；官方无 Key 级余额接口，明确提示到百炼控制台查看账户余额/费用
  - `info sale-price alibaba-cn`：给出百炼官方模型列表与计费说明页地址（未接入自动抓取）
  - `info price alibaba-cn`：models.dev 已有 `alibaba-cn`（Alibaba (China)，91 个模型）条目，直接命中
- 版本 0.4.15 -> 0.4.16

## v0.4.15（2026-10-07）

### 新增
- **`sys ai info` 新增两个参数**（`sai info`）：
  - `info balance PROVIDER`：用该 provider 的 `api_key` 到官方接口查询余额。minimax 走官方 `token_plan/remains`（Token Plan 剩余额度；非订阅用户给出按量付费余额入口）；agnes 走 OpenAI 兼容 billing 探测（credit_grants / subscription / usage，无余额字段时提示控制台）；modelscope / sensenova / bigmodel / amd / anspire 官方未开放 Key 级余额接口，命令给出对应控制台地址。余额查询实时进行，不走缓存
  - `info sale-price PROVIDER`：抓取官网定价页并列出全部模型销售价。agnes 抓取 `wiki.agnes-ai.cn` 定价页（文本/图片/视频模型，刊例价+现价，人民币）；minimax 抓取 `platform.minimaxi.com` 定价页（语言模型输入/输出/缓存价格，元/百万 tokens）；其余 provider 给出官网定价页地址。定价页 24h 缓存，`--refresh` 强制重抓
- 版本 0.4.14 -> 0.4.15

## v0.4.14（2026-10-07）

### 新增
- **`sys ai info`（`sai info`）**：检查本地配置与模型价格
  - `info provider [KEYWORD]`：列出配置文件 `clients` 中全部 provider（`name` / `api_base` / `api_key`）；带 KEYWORD 时只保留名称包含该词的 provider（大小写不敏感），无匹配则报错并列出可用名称
  - `info model [KEYWORD]`：列出全部 provider 下的 model，格式为 `{provider}:{name}`；带 KEYWORD 时按 provider 名称包含匹配（支持 `provider:model` / `provider:*` 精确选择）；无 provider 命中时回退按 model 名称匹配
  - `info price P1,P2,...`：按逗号分隔的 provider 名称（models.dev id/名称，支持中英文逗号）查询模型价格列表，输出 input / output / cache_read 每 1M token 价格，数据源 models.dev（复用 24h 缓存，`--refresh` 强制刷新；`-c/--config` 可覆盖配置文件路径）
- 版本 0.4.13 -> 0.4.14

## v0.4.13（2026-10-06）

### 新增
- **`sys search --ai`**：接入博查 **AI Search API**（`POST https://api.bochaai.com/v1/ai-search`）高级搜索
  - `--ai` 切换端点：在网页搜索基础上额外返回垂域结构化模态卡（天气/百科/日历/股票等）与 **AI 实时生成的答案**（`answer` 默认开启；`--no-answer` 关闭，仅可与 `--ai` 同用）；`stream` 固定关闭（纯 CLI 调用）
  - 请求参数与官网接口一致：`query` / `freshness` / `count` / `page` / `include_domains` / `exclude_domains` / `answer` / `stream`；输出默认含 AI 答案 + 逐条摘要/站点/发布时间；`--json` 原始响应、`--debug` 打印实际请求与响应
  - 客户端重构：共用 `bocha_request`（POST + 信封校验），响应解析兼容 `data.messages[]`（source 结果 / answer 答案）、`data.webPages.value[]`、`data.web_results[]` 三种形态，标题兼容 `name`/`webpage`/`title` 字段、时间兼容 `datePublished`/`dateLastCrawled`/`page_timestamp`
  - 注意：**AI Search 与 Web Search 是独立套餐**。当前配置 key 仅有 Web Search 套餐，`--ai` 请求会得到 403（`You do not have enough money or package quota`），需在博查开放平台为 key 开通 AI Search 包；请求构造与响应解析已通过确定性测试覆盖（真实响应结构参照官网文档）
- 版本 0.4.12 -> 0.4.13

## v0.4.12（2026-10-06）

### 新增
- **`sys ai image`（`sai image`）**：通过 Provider 的 OpenAI 兼容 **Images API（`POST {api_base}/images/generations`）** 生成图片
  - 与 `ai chat` 共用 `~/.sysenv/config.yaml` 的 `clients` 配置与模型选择规则：`-m/--model` 支持 `{provider}:{model}` / `{model}` / 逗号分隔多选，多模型目标按 `weight` 加权轮询，失败自动降权并尝试下一个模型（成功替补升权，写回配置文件）
  - `-o/--output DIR` 指定保存目录（默认当前目录）；`-n/--count N` 数量（默认 1）；`-s/--size SIZE` 尺寸（默认 `1024x1024`，原样传给 API）；`--url` 改为请求图片 URL 并下载保存（默认请求 `b64_json` 本地解码保存）
  - 图片保存为 `sys-ai-image-<时间戳>-<序号>.<格式>`，扩展名按文件头魔数识别（png / jpg / gif / webp），文件名冲突自动加 `-1`、`-2` 后缀；每张图片的保存路径打印到 stdout
  - 仅支持 OpenAI 兼容 Provider（`type: openai`）；`type: anthropic` 目标在发请求前明确报错；`--list-provider / --list-model` 与 `ai chat` 一致
  - 冒烟验证：请求正确到达智谱 `/images/generations`（429 余额不足系账号余额问题，接口与错误透传正常）；agnes 对话模型返回 400 并提示"Use /v1/chat/completions"
- **`sys search`（`ssearch`）**：接入**博查 AI 网页搜索 API**（`POST https://api.bochaai.com/v1/web-search`）提供联网搜索
  - API key 读取配置顶层 `search:` 段（如 `- name: bochaai, key: sk-...`），`-c/--config` 可覆盖默认 `~/.sysenv/config.yaml`
  - **参数与官网接口一致**：`query`（位置参数，多词自动空格连接，支持 stdin 管道）、`--freshness`（`noLimit`/`oneDay`/`oneWeek`/`oneMonth`/`oneYear`/`YYYY-MM-DD`/区间）、`--summary`（返回 AI 摘要与逐条摘要）、`--count`（1-50，默认 10）、`--page`（默认 1）、`--include-domains` / `--exclude-domains`（域名白/黑名单，可重复）
  - 默认输出标题+链接行；`--summary` 追加摘要/站点/发布时间；`--json` 输出原始响应（含 AI 摘要与分页）；`--debug` 打印实际请求与响应
  - 同时注册为内置搜索源 `bochaai`（`ai task --list-source` 可查）：任务可用 `search: bochaai` 联网搜索，`q`/`query` 为查询词，`freshness`/`count`/`page`/`summary`/`include_domains`/`exclude_domains` 为可选参数；`short` 新增 `ssearch` shim
  - 冒烟验证：真实 key 搜索「2026年诺贝尔物理学奖」「深圳今天天气」返回真实网页标题/链接/摘要与站点信息，`--debug` 请求体含 `freshness`/`count`/`include_domains` 等官网参数

## v0.4.11（2026-10-05）

### 变更
- **搜索源精简**：删除 `zhihu` / `baidu` / `bing` / `dxtower` / `cls`（财联社）/ `penalty` / `company` / `tophub` 共 8 个源，保留通用聚合源 `sogou`；`ai task --list-source` 现 **11 个源**（bilibili / github / hn / toutiao / oschina / smzdm / sogou / enlightent / dongchedi / autohome / szhousing）
- **任务精简**：删除 `penalty` / `company` / `finance` / `technews` 任务，`ai task` 现 **8 个任务**（weather / cloudrank / hotnews / auto / cartech / oschina / deals / szhousing）
- **szhousing 重构（纯 HTTP 直连）**：移除 Chrome/CDP 浏览器方案，改为直接请求官方 API —— 实测平台 API（`/szfdcscjy/*`）不经瑞数校验（瑞数只挂 HTML 首页），带常规 Chrome UA + Origin + Referer 头 POST 即可 200；**无需浏览器、不依赖本机任何软件**，Windows/Ubuntu 完全一致；新增楼栋**备案均价**统计；单次查询由 ~5s 降至 <1s
- `ai cn-model` 新增 `--list` 参数：列出全部 1015 个模型（`--limit` 可截断），无参报错文案改为 `provide a model NAME, --search QUERY, or --list`

## v0.4.10（2026-10-05）

### 修复
- **hotdrama / cloudrank 搜索失效**：修复"今天电视剧热度排行榜"等请求工具结果跑偏的问题
  - 根因：德塔文/云合等专名被搜索引擎错误拆词（如"德塔文"→"德"），返回汉字字典、高德地图等无关结果；实测德塔文官网（dxtower.com）无法直连
  - 修复：`dxtower` 源改用通用热度词 `电视剧 热度 排行榜 今日`、`enlightent` 源改用 `电视剧 热播 榜单 今日`，命中爱奇艺/腾讯视频/央视等平台热播剧频道页，模型据此总结今日热播剧榜单
  - hotdrama / cloudrank 任务 desc 同步改为"电视剧热度排行榜/热播榜（AI 聚合多源搜索）"
- 冒烟验证：`-t hotdrama` 工具回填真实平台热播剧频道结果，无字典/百科/地图污染

## v0.4.9（2026-10-05）

### 修复
- **AI 聚合搜索结果净化**：修复搜索结果大量掺杂百度百科等低价值词条导致搜索质量差/失败的问题
  - 新增 `is_low_value_row` 过滤：标题含「百度百科/搜狗百科/维基百科/互动百科」或 URL 命中 `baike.baidu.com` / `zh|en.wikipedia.org` / `baike.sogou.com` / `baike.com` 的行直接剔除（标题通道覆盖 Sogou `/link?url=` 重定向隐藏真实域名的情况）
  - Sogou 请求失败不再直接报错，自动降级 Bing 兜底；两引擎均无可过滤结果时合并原始结果去重兜底，任务不会因过度过滤而失败
- 新增单测 `low_value_rows_are_detected`（79 项测试全绿）

## v0.4.8（2026-10-05）

### 新增
- `sys ai task --list-source`：列出全部内置搜索源的**名称、类型（api 直连数据接口 / search AI 聚合搜索 Sogou 优先 Bing 兜底 / generic 通用搜索需 q/query 参数）、用途与访问地址**，共 19 个源（zhihu/baidu/bilibili/github/hn/toutiao/tophub/oschina/smzdm/bing/sogou/dxtower/enlightent/cls/dongchedi/autohome/szhousing/penalty/company）

## v0.4.7（2026-10-05）

### 新增
- `sys ai task` 新增两个搜索源：
  - `penalty`：查询**某人/某单位的行政处罚、失信被执行人信息**（公开公示渠道，AI 聚合搜索）
  - `company`：查询**公司工商注册信息**（法定代表人、注册资本、成立日期等，公开公示渠道，AI 聚合搜索）
- 实现说明：实测信用中国（creditchina.gov.cn）无响应、国家企业信用信息公示系统（gsxt.gov.cn）521、中国执行信息公开网/裁判文书网/爱企查均需验证码或 JS 渲染、无法程序直连 → 采用 AI 聚合搜索（Sogou 优先、Bing 兜底）抓取公开公示页面（企查查/爱企查/百科/政府公示等）回填，模型如实总结；查询对象由模型的 `name` 参数指定，无相关记录时模型如实说明
- **`-t` 手动指定任务时自由文本参数自动填充任务的第一个声明参数**（如 `-t company 字节跳动` → `name=字节跳动`，msg 中 `{name}` 占位符与工具 query 均被替换），`weather` 等既有任务不受影响
- 配置示例新增 `penalty` / `company` 任务（`~/.sysenv/config.yaml` 与 `doc/config.yaml`）

## v0.4.6（2026-10-05）

### 新增
- `sys ai task` 新增搜索源 `szhousing`：获取**深圳房源销售情况**（新房/二手房成交套数等，深圳房地产信息平台公开数据）
- 实现说明：`fdc.zjj.sz.gov.cn` 经实测部署瑞数动态 WAF（curl 全量浏览器头仍返回 HTTP 412 验证页），无法程序直连；改用 AI 聚合搜索（Sogou 优先、Bing 兜底）抓取公开渠道（乐有家/中原/住建局官网等）的成交数据报道回填，模型如实总结并注明来源
- 配置示例新增任务 `szhousing`（`~/.sysenv/config.yaml` 与 `doc/config.yaml`）

## v0.4.5（2026-10-05）

### 新增
- `sys ai task` 新增 **10 个内置搜索源**，覆盖用户点名的 5 组数据源：
  - **热播电视剧**：`dxtower`（德塔文电视剧景气指数）、`enlightent`（云合数据霸屏榜/热播榜）——目标站点接口带签名/WAF，改用 AI 聚合搜索实现
  - **热门财经及新闻媒体**：`toutiao`（今日头条热榜，JSON 直连）、`cls`（财联社电报，AI 聚合搜索）
  - **汽车新闻**：`autohome`（汽车之家）、`dongchedi`（懂车帝）——AI 聚合搜索
  - **技术类新闻**：`tophub`（tophub.today 开发者技术榜，SSR 直连）、`oschina`（开源中国资讯，SSR 直连）
  - **特价商品**：`smzdm`（什么值得买今日好价，SSR 直连，标题+价格）
- **AI 聚合搜索引擎**：Sogou 优先（中文分词可靠）、Bing 兜底（Sogou 触发验证码/空结果时自动回退），程序抓取搜索结果标题+链接回填给模型，模型如实筛选总结
- `-t NAME` 手动指定任务时，若任务声明了 `api`/`search` 工具，同样**先执行工具并回填真实数据**再回答（此前仅 function_call 自动路由会执行工具）
- 配置示例（`~/.sysenv/config.yaml` 与 `doc/config.yaml`）新增 `hotdrama` / `cloudrank` / `hotnews` / `finance` / `auto` / `cartech` / `technews` / `oschina` / `deals` 共 9 个任务

### 实现说明
- 所有新搜索源均为纯代码 HTTP 请求（reqwest + 浏览器 UA），**无本地 shell 命令**；HTML 页面用内置解析器提取（含标签剥离、HTML 实体解码、URL 百分号编码工具）
- 德塔文（dxtower.com）/ 云合（enlightent.cn）官网接口经实测无法直连（WAF 拒绝连接 / 榜单数据接口签名），财联社数据接口全签名、懂车帝/汽车之家首页为 JS 异步渲染——故这些站点采用 AI 聚合搜索方式：抓取 Sogou/Bing 的相关搜索结果回填，模型基于真实结果筛选总结，数据不足时如实说明

## v0.4.4（2026-10-05）

### 新增
- `sys ai task` **任务工具执行改为纯代码 HTTP 实现，禁止本地 shell 命令**（移除 `tool:` 命令模板，删除 `run_tool_cmd`）：任务可声明两种工具，function_call 选中后由程序执行并**回填真实数据**给模型回答：
  - **`api: <URL模板>`** —— 固定 HTTP 接口：`{key}` / `{key:默认值}` 占位符由模型 tool 参数填充（空值回退默认值），响应体原样作为工具结果；`params: [k1, k2]` 声明可选参数并加入函数 schema
  - **`search: <源名>`** —— AI 模糊联网搜索：内置 `zhihu`（知乎日报热门）、`baidu`（百度实时热搜）、`bilibili`（B 站热门）、`github`（近 7 天新建星榜，可传 `date` 参数）、`hn`（Hacker News）五个搜索源，程序抓取头条列表回填，模型据此筛选总结
- 自研 YAML 解析器支持内联数组 `[a, b]`（`params: [lat, lon]` 与块列表写法均可）
- 配置示例新增 `topnews`（zhihu）与 `topshow`（baidu）任务，weather 改用 `api:` 写法

### 修复
- 模型 function_call 参数传空字符串时，URL 模板占位符回退使用默认值（此前空值会生成无效 URL）

## v0.4.3（2026-10-05）

### 新增
- `sys ai task` 无 `-t` 且带用户请求（参数或 stdin）时改为 **function_call 自动路由**：
  - 每个任务的 `name` 作为函数名、`desc` 作为函数描述注册为 tools（OpenAI / Anthropic 双协议）
  - 大模型用 function_call 选择最匹配的任务，用户请求通过 `input` 参数传入
  - 自动执行选中的任务：把任务 `desc` + 用户请求组装成消息发给模型，输出最终回复
  - 模型未选择任何任务时明确报错，提示改用 `-t NAME` 手动指定
- 兼容与保留：无输入仍列出任务；`-t NAME`（模板占位符替换）与 `-t *`（0-10 分打分匹配）行为不变；路由请求同样走动态权重轮询与失败自动切换

## v0.4.2（2026-10-05）

### 新增
- `sys http` 新增 **`--file FILE`** 参数：读取本地文件内容作为原始请求体（等价位置参数 `@FILE`），自动判定为 POST；与 `@FILE` 位置参数/`--raw` 互斥并明确报错
- `sys http` 请求项字段值以 `@` 开头（`key=@file`、`key:=@file`、`key==@file`、`key:@file`）读取本地文件作为该字段值：**自动剥离 UTF-8 BOM 与尾部换行**，文件内容中的引号、反斜杠等特殊符号在 JSON 序列化时**自动转义**

## v0.4.1（2026-10-05）

### 变更
- `sys http` **默认采用 `application/json`**：所有请求默认携带 `Content-Type: application/json`（包括无数据项、GET/POST 等任何方法），`Accept` 默认 `application/json, */*;q=0.5`；可用 `-f/--form`、`--multipart` 或显式 `Content-Type:xxx` 请求头覆盖

## v0.4.0（2026-10-05）

### 新增
- `sys ai model` / `ai cn-model` 的 `--model-type` 支持**半角 `,` / 全角 `，` 逗号分隔的多个类型**，取**交集**：只显示同时包含全部指定模态的模型（如 `--model-type text,image` 只显示既能文本又能图像的模型；中文别名 `文本，图像` 同样可用）
- `sys ai chat` / `ai task` 发送 HTTP 请求前自动补齐模型能力字段：模型项缺少 `max_input_tokens` 或 `type` 时，从 models.dev 按模型名称查询匹配的第 1 个模型，用其 `context` 填充 `max_input_tokens`、用其输入/输出模态并集填充 `type`，并**写回配置文件**（下次直接使用）；查询失败或无匹配时静默跳过
- **消息长度截断**：发送前检查消息字符数是否超过该模型 `max_input_tokens`，超过则截取到限制内并在 stderr 提示，防止超出上下文窗口
- `sys ai task -t *`：**全任务打分匹配**——把所有任务的 `desc` 与用户提供的聊天信息组装，调用配置的大模型按 10 分制打分（0=不匹配，10=完全匹配），输出 `TASK / DESC / SCORE` 表格并按分数降序；单任务失败时 SCORE 显示 `-`

### 说明
- 配置文件模型项新增可选字段 `max_input_tokens`（字符上限）与 `type`（模态并集，如 `text,image`），也可手动预填
- `-t *` 在 PowerShell 等 shell 中需加引号（`-t "*"`）避免通配符展开

## v0.3.0（2026-10-05）

### 新增
- `sys ai chat` 新增 `--list-model`：列出配置文件 `clients` 下**所有模型名称**（按 Provider 分组，含权重）；新增 `--list-provider`：列出所有 Provider（名称 / type / 模型数）；两者可同时使用，无需消息
- `sys ai chat` 新增 `-m/--model MODEL`：**临时覆盖**顶层 `model`，规则与配置完全一致（`{provider}:{model}` / `{provider}:*` / 裸 `{model}` / 逗号分隔多选）
- 顶层 `model` 与 `-m` 均支持**半角逗号 `,` / 全角逗号 `，` 分隔的多个模型选择**，按权重轮询使用（如 `agnes:*,claude:claude-3-5-sonnet`）
- **动态权重轮询**：模型 `weight` 取值范围 0-9（缺省 1）；请求失败时若 `weight > 1` 则 `-1` 并**写回配置文件**，自动尝试下一个模型；替补模型成功且 `weight < 9` 则 `+1` 并写回（同时兼容 `- name: X` 下常规 `weight:` 子键与 mangled 独立 `- weight: N` 两种写法，保留注释与缩进）
- `sys ai model` 文本列表新增 `CONTEXT`（上下文长度）与 `TYPE`（模态类型）列；新增 `--model-type TYPE` 筛选：`text / image / audio / video / pdf`（支持中文 `文本 / 图像 / 语音 / 视频`），按 `modalities.input/output` 判定
- `sys ai cn-model` 列表新增 `TYPE`（分类）列；新增 `--model-type TYPE` 筛选：`text / image / audio / video / multimodal`（支持中文），按卡片分类匹配（如 `audio` → 语音大模型，`image` → 视觉 / 多模态类）；**精确命中单模型时抓取详情页**，输出附带 `context`（上下文长度，如 `1.05M`）与 `modality`（输入/输出模态，如 `文本、图像 → 文本`），失败静默降级；CSV 扩展为 10 列（新增 `context,modality`）
- 健壮性：配置文件解析自动剥离 UTF-8 BOM，避免顶层 `model` 字段静默丢失

### 说明
- 旧版 `chat_state.json` 轮询状态兼容（selector 含逗号时以整体字符串为状态键）
