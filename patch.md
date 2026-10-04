# sysenv 更新日志（Patch Notes）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sysenv-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

---

## v0.2.8（2026-10-04）

### 新增
- `sysenv task`（`stask`）新增 `-o/--port <PORT>`：查看**占用指定端口的进程**（与 `list` 的名称 / PID 过滤可组合；`kill` 模式不适用）；Windows 用 `netstat -ano`，Linux 用 `ss -ltnp`

---

## v0.2.7（2026-10-04）

### 新增
- `sysenv task`（`stask`）不加参数时默认列出**全部进程**（等价于 `sysenv task list`），与 `sai model` 无参默认全量的行为保持一致

---

## v0.2.6（2026-10-04）

### 变更
- `sysenv ai model` 文本列表（无参 / `--list` / `--search` / 多匹配）默认展示改为**带列名对齐表格**：`ID  NAME  FAMILY  LAST UPDATED`（原先为无表头的 `id 名称 provider` 制表符行）；单模型详情视图（`== 标题 ==` + 字段行）不变

---

## v0.2.5（2026-10-04）

### 新增
- `sysenv ai model --date YYYY-MM-DD`：由 v0.2.4 的 `--data` 更名定稿（功能不变：仅显示 `last_updated` 晚于该日期的模型）
- `sysenv ai model --open`：仅显示 `open_weights: yes` 的模型；可与 `--date` / 无参全量 / `--list` / `--search` / 名称查询组合（先按日期、再按开放权重过滤）
- 新增 `sysenv task` 子命令（Windows / Ubuntu）：
  - `task list [NAME]`：列出进程的 PID / 名称 / 可执行文件路径；NAME 按名称模糊匹配（子串、大小写不敏感），传数字则按 PID 精确查询
  - `task kill <PID|名称>`：按 PID 或名称终止进程；名称模糊匹配会终止全部命中进程；`-f/--force` 强制终止（Linux 发送 SIGKILL，默认 SIGTERM）；无权限等失败项单独提示
- 快捷垫片新增 `stask`（`sysenv task`），`sysenv short` 一次性安装 6 个短命令

---

## v0.2.4（2026-10-04）

### 新增
- `sysenv ai model`（`sai model`）不加参数时默认列出**全部模型**（原为报错提示需提供名称或 `--search/--list`）；`--limit N` 仍可限制条数，`--list` 保持默认 20 条分页
- `sysenv ai model --data YYYY-MM-DD`：仅显示 `last_updated` **晚于**该日期（严格大于）的模型；可与 `--list` / 无参全量 / `--search` / 名称查询组合（先按日期过滤再匹配）；日期格式非法时给出明确报错

---

## v0.2.3（2026-10-03）

### 新增
- `-V` / `--version` 版本信息增加署名：`sysenv 0.2.3 (Made by Gary-china)`（版本号自动跟随 Cargo.toml）

---

## v0.2.2（2026-10-03）

### 修复
- `sysenv ai model` 详情输出（单命中 / canonical 优选）缺少标题：现在顶部增加标题行 `== <模型标题> ==`，取自 models.dev 页面上的模型标题（即模型 `name` 字段，缺失时回退为 `id`），例如 `sai model deepseek-chat` 首行输出 `== DeepSeek V3/Deepseek Chat ==`
- 多匹配列表 / 搜索 / 列表模式不受影响（每行已含标题列 `name`）

---

## v0.2.1（2026-10-03）

### 新增
- `sysenv ai model` 与 `sysenv ai provider` 增加 `-o/--output-format json|csv` 可选参数：
  - `-o json`：输出 **JSON 数组**格式（与 `--json` 等价，单命中也是数组；适用于详情 / 搜索 / 列表 / 多匹配全部场景）
  - `-o csv`：输出带表头的 CSV 表格（RFC-4180 转义）——model 24 列、provider 6 列
- 非法格式值（如 `-o xml`）由 clap 校验拒绝并提示可选值 `json, csv`；`--json` 与 `-o` 互斥

### 发布规范
- release 产物统一使用 **UPX 压缩**（实测：Windows 5.1 MB → 1.79 MB，Linux 5.1 MB → 1.91 MB）
- 产物文件名包含平台 + 架构 + 版本号：
  - `sysenv-windows-x86_64_v0.2.1.exe`
  - `sysenv-linux-x86_64_v0.2.1`
- 新增本文件（`patch.md`），记录每次版本号变更的新增 / 修复内容

---

## v0.2.0（2026-10-03）

### 新增
- `sysenv ai` 子命令：查询 models.dev（`https://models.dev/api.json`，226 个 Provider、8000+ 模型）
  - `ai model NAME`：按模型名称 / id / canonical id 精确查询；多 Provider 同名时自动优选 canonical 条目（如 `gpt-4.1` → OpenAI）并提示其余 Provider 数量
  - `ai provider NAME`：按 Provider id / 名称查询（api / npm / 模型清单）
  - `-s/--search` 子串搜索（grep 风格列表）、`--list` 分页浏览、`--limit` 条数控制（默认 20）、`--json` 原始输出、`--refresh` 强制重抓
  - 24 小时本地缓存（Windows `%LOCALAPPDATA%\sysenv\`，Linux `$XDG_CACHE_HOME` 或 `~/.cache/sysenv/`）
- `sysenv short` 子命令：一键安装全部子命令的快捷垫片 `spath` / `senv` / `slink` / `shttp` / `sai`（Windows `.cmd`、Linux 可执行 sh 脚本），自动持久化加入 PATH，支持 `--dir` / `--force` / `--temporary`

### 修复
- 修正 ai 查询匹配逻辑：`--search` 改为列表输出；裸名称多 Provider 匹配时优先 canonical 条目

---

## v0.1.0（2026-10-03）

### 新增（初始版本）
- `sysenv path`：PATH 增删查改（`list` / `add` / `remove` / `has`），自动持久化并同时作用于当前环境；`-p` 前置、`--scope user|machine` 双作用域、`--temporary` 临时模式；自动转绝对路径、去重
- `sysenv path export/import`：PATH 与 Windows 注册表文件（`.reg`）、JSON、文本双向导入导出，支持合并与 `--replace` 整体替换
- `sysenv link`：把任意文件链接进 PATH 目录（硬链接 → 符号链接 → 拷贝自动回退；Windows 自动生成 `.cmd` 垫片；`--name` / `--method` / `--system` / `--dir`）
- `sysenv env`：环境变量 `get` / `set` / `unset` / `list`，默认持久化，`--temporary` 仅当前 shell，`--scope machine`
- `sysenv http`：httpie 兼容 HTTP 客户端（`-j/-f/--multipart/--raw`、`-p/-h/-b/-m/-v/-o/-d/-q`、`-a/-A` 认证、`-F/--follow/--max-redirects/--timeout/--proxy/--check-status/--offline/--verify/-I/--default-scheme`；请求项语法：`key=value`、`key:=json`、`key==value`、`key:value`、`key@file`、`@file`、嵌套 JSON）
