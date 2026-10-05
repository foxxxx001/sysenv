# sysenv 更新日志（CHANGELOG）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sysenv-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

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
