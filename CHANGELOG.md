# sysenv 更新日志（CHANGELOG）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sysenv-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

## v0.4.2（2026-10-05）

### 新增
- `sysenv http` 新增 **`--file FILE`** 参数：读取本地文件内容作为原始请求体（等价位置参数 `@FILE`），自动判定为 POST；与 `@FILE` 位置参数/`--raw` 互斥并明确报错
- `sysenv http` 请求项字段值以 `@` 开头（`key=@file`、`key:=@file`、`key==@file`、`key:@file`）读取本地文件作为该字段值：**自动剥离 UTF-8 BOM 与尾部换行**，文件内容中的引号、反斜杠等特殊符号在 JSON 序列化时**自动转义**

## v0.4.1（2026-10-05）

### 变更
- `sysenv http` **默认采用 `application/json`**：所有请求默认携带 `Content-Type: application/json`（包括无数据项、GET/POST 等任何方法），`Accept` 默认 `application/json, */*;q=0.5`；可用 `-f/--form`、`--multipart` 或显式 `Content-Type:xxx` 请求头覆盖

## v0.4.0（2026-10-05）

### 新增
- `sysenv ai model` / `ai cn-model` 的 `--model-type` 支持**半角 `,` / 全角 `，` 逗号分隔的多个类型**，取**交集**：只显示同时包含全部指定模态的模型（如 `--model-type text,image` 只显示既能文本又能图像的模型；中文别名 `文本，图像` 同样可用）
- `sysenv ai chat` / `ai task` 发送 HTTP 请求前自动补齐模型能力字段：模型项缺少 `max_input_tokens` 或 `type` 时，从 models.dev 按模型名称查询匹配的第 1 个模型，用其 `context` 填充 `max_input_tokens`、用其输入/输出模态并集填充 `type`，并**写回配置文件**（下次直接使用）；查询失败或无匹配时静默跳过
- **消息长度截断**：发送前检查消息字符数是否超过该模型 `max_input_tokens`，超过则截取到限制内并在 stderr 提示，防止超出上下文窗口
- `sysenv ai task -t *`：**全任务打分匹配**——把所有任务的 `desc` 与用户提供的聊天信息组装，调用配置的大模型按 10 分制打分（0=不匹配，10=完全匹配），输出 `TASK / DESC / SCORE` 表格并按分数降序；单任务失败时 SCORE 显示 `-`

### 说明
- 配置文件模型项新增可选字段 `max_input_tokens`（字符上限）与 `type`（模态并集，如 `text,image`），也可手动预填
- `-t *` 在 PowerShell 等 shell 中需加引号（`-t "*"`）避免通配符展开

## v0.3.0（2026-10-05）

### 新增
- `sysenv ai chat` 新增 `--list-model`：列出配置文件 `clients` 下**所有模型名称**（按 Provider 分组，含权重）；新增 `--list-provider`：列出所有 Provider（名称 / type / 模型数）；两者可同时使用，无需消息
- `sysenv ai chat` 新增 `-m/--model MODEL`：**临时覆盖**顶层 `model`，规则与配置完全一致（`{provider}:{model}` / `{provider}:*` / 裸 `{model}` / 逗号分隔多选）
- 顶层 `model` 与 `-m` 均支持**半角逗号 `,` / 全角逗号 `，` 分隔的多个模型选择**，按权重轮询使用（如 `agnes:*,claude:claude-3-5-sonnet`）
- **动态权重轮询**：模型 `weight` 取值范围 0-9（缺省 1）；请求失败时若 `weight > 1` 则 `-1` 并**写回配置文件**，自动尝试下一个模型；替补模型成功且 `weight < 9` 则 `+1` 并写回（同时兼容 `- name: X` 下常规 `weight:` 子键与 mangled 独立 `- weight: N` 两种写法，保留注释与缩进）
- `sysenv ai model` 文本列表新增 `CONTEXT`（上下文长度）与 `TYPE`（模态类型）列；新增 `--model-type TYPE` 筛选：`text / image / audio / video / pdf`（支持中文 `文本 / 图像 / 语音 / 视频`），按 `modalities.input/output` 判定
- `sysenv ai cn-model` 列表新增 `TYPE`（分类）列；新增 `--model-type TYPE` 筛选：`text / image / audio / video / multimodal`（支持中文），按卡片分类匹配（如 `audio` → 语音大模型，`image` → 视觉 / 多模态类）；**精确命中单模型时抓取详情页**，输出附带 `context`（上下文长度，如 `1.05M`）与 `modality`（输入/输出模态，如 `文本、图像 → 文本`），失败静默降级；CSV 扩展为 10 列（新增 `context,modality`）
- 健壮性：配置文件解析自动剥离 UTF-8 BOM，避免顶层 `model` 字段静默丢失

### 说明
- 旧版 `chat_state.json` 轮询状态兼容（selector 含逗号时以整体字符串为状态键）
