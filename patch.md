# sys 更新日志（Patch Notes）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sys-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

## v0.4.66（2026-10-11）

### 新增
- **`sys ai chat --test`：并行聊天反馈时间测试**。向选中的模型（默认全部配置模型；`-m MODEL` 指定，支持 `provider:model` / `provider:*` / 逗号列表）发起 streaming 聊天，**只测到首个 token 的反馈时间、不等完整回复**（收到首个非空内容增量即断开连接）：
  - 聊天内容默认 `hello`，也可直接传入 MSG 作为测试内容
  - 并行执行，默认并行数 = 检测到的 CPU 核心数，可用 `--parallel N` 指定（自动限制在 1..模型数）
  - 成功打印 `[OK] provider / model  first token in X.XXs`；失败（HTTP 错误如 429、连接错误、超时、无内容反馈）打印 `[FAIL] ...` 并将该模型配置的 `weight` 写为 0（串行写回避免并发改配置；已是 0 则不重写；同一 provider 下的同名模型条目按出现次序精确定位——`update_weight_in_config_at` 新增 occurrence 参数），末尾汇总 `ok/N model(s) responded`
  - 权重写回失败仅警告、不中断；`--test` 不读 stdin、不走 `--debug`/`--update`/`--no-stream` 路径
- 新增 6 个单测（全部/`-m`/`provider:model` 目标解析；本地 TCP SSE 服务器验证首 token 计时跳过空增量、HTTP 429 失败、stream 内 error 失败、无内容流失败；同名模型条目按 occurrence 定位）；`cargo test` 227 项全绿
- 版本 0.4.65 -> 0.4.66

---

## v0.4.65（2026-10-11）

### 变更
- `sys ai info provider` 每条信息新增 **`sample`** 字段：用该 provider 的 `api_base` / `api_key` 对其 models 接口的示例请求
  - OpenAI 兼容：`https://{api_base}/models  Authorization:"Bearer {api_key}"`（如 `https://openrouter.ai/api/v1/models  Authorization:"Bearer sk-sample"`）
  - Anthropic：`x-api-key:"{api_key}" anthropic-version:"2023-06-01"`
  - 出现在默认 YAML 视图、`-o md` 列表（`sample：` 子项）、`-o json` / `-o csv`（新增 `sample` 列）与无字段组合视图的 JSON/YAML 中
- 新增 2 个单测（md 列表 sample 行、anthropic x-api-key sample）；`cargo test` 221 项全绿

---

## v0.4.64（2026-10-11）

### 变更
- `sys ai info` 的 **`-o md` 输出从 Markdown 表格改为无序列表**：
  - `info provider -o md`：
    ```
    - agnes
      - api_base：https://apihub.agnes-ai.cn/v1
      - api_key：sk-...
      - console：https://platform.agnes-ai.cn
      - docs：https://wiki.agnes-ai.cn/zh-Hans/docs
    ```
  - `info model -o md`：按 provider 分组，`- provider` 下挂 `- name：模型` 子项
  - 无字段组合视图：`## Providers` / `## Models` 两段无序列表
  - `info list-model -o md`：`- 模型id` 下挂 `owned_by：` / `created：` 子项
- `sys ai model` / `ai provider` / `ai cn-model` / `ai dl` 的 `-o md` 仍为 Markdown 表格，不受影响
- 测试同步更新（`cargo test` 220 项全绿）

---

## v0.4.63（2026-10-11）

### 变更
- `sys ai info`（及 `sys ai model` / `sys ai provider` / `sys ai dl` / `info list-model`）新增 **`-o md`**：输出 Markdown 表格（`| 表头 | ... |` + `| --- |` 分隔行 + 数据行；单元格转义 `|` 与换行）
  - `info provider -o md`：`| name | api_base | api_key | docs | console |`
  - `info model -o md`：`| provider | name |`；无字段组合视图输出 `## Providers` / `## Models` 两段表格
  - `info list-model -o md`：`| id | owned_by | created |`
  - `model`/`provider`/`dl` 列表视图输出 id/name/provider 等简化表格；单条详情输出 `| key | value |`
- 新增 2 个单测（md 表格渲染与转义、info provider md 表）；`cargo test` 220 项全绿

---

## v0.4.62（2026-10-11）

### 变更
- `sys net -t tv` / `-t tvs` 新增 **`--words KEYWORD`**（默认 `4k`）：统计 `.sites.name` 中含该关键词（大小写不敏感）的节点数量；屏幕与 `-o` 输出的 `name` 字段改为显示关键词本身，如默认时 `4k=17`、指定 `--words hd` 时 `hd=...`
- 内部 `count_name_4k` 参数化为 `count_name_containing(v, word)`；新增 cmd 级测试（默认 4k 标签、自定义词标签与计数、无残留 4k 标签）；`cargo test` 218 项全绿

---

## v0.4.61（2026-10-11）

### 变更
- 修复：带 **UTF-8/UTF-16 BOM** 前缀的 JSON 响应被误判 `[FMT] body is not valid JSON`（如 raw.githubusercontent 以 `text/plain` 提供 BOM 前缀 JSON，内容实际是合法的多仓 `urls` 格式）——`parse_tv_json` 解析前先剥离 BOM，tv / tvs 均受益
- 新增单测（BOM 前缀的多仓与单仓 JSON 均可解析）；`cargo test` 217 项全绿

---

## v0.4.60（2026-10-11）

### 变更
- `sys net -t tv` / `-t tvs`：输入行中地址后面（隔空格）的 `#` 内容作为**备注**随地址输出——逐条 `[OK]`/`[FMT]`/`[NO]` 行末尾 `note=...`，最终排序/分组输出与 `-o` 文件的 `#` 测试信息行显示 `note=...  count=...`；备注取自该行最后一个地址，无备注时输出格式与旧版一致
- 输入侧重构：新增 `read_entries`（返回 url + 备注对），`read_addresses` 复用之；`probe_parallel` 携带备注；tv / tvs 输出路径全量支持
- 新增 3 个单测（行尾备注解析、tv 输出含 note、tvs 输出含 note）；`cargo test` 216 项全绿

---

## v0.4.59（2026-10-11）

### 变更
- `sys net -t info` 补全注册主体：registry 层（Verisign 等）的 RDAP 响应常隐藏 registrant 实体，但响应中的 `related`（application/rdap+json）链接指向注册商级 RDAP——现在**自动跟随该链接**并用更完整的实体数据（如 `www.163.com` → 回退 163.com → Verisign 无主体 → 跟随 MarkMonitor → 输出 `Org: 广州网易计算机系统有限公司`）
- 国家/地区提取增强：优先取 vCard `adr` 行的 `cc` 参数（MarkMonitor 风格 `{"cc":"CN"}`），其次值数组第 7 位，最后兜底最后一个非空组件
- 新增 2 个单测（adr cc 参数国家提取、related 链接跟随双服务器 mock）；`cargo test` 213 项全绿

---

## v0.4.58（2026-10-11）

### 变更
- `sys net -t tv` / `-t tvs` 改为**多地址并行测试**：新增 `--thread NUM` 指定一次并行探测几个地址（默认 **CPU 内核数量**）；每个地址测完立即在屏幕打印一条 `[OK]` / `[FMT]` / `[NO]` 结果（含状态码与响应时间），全部完成后输出原有排序/分组结果与汇总行；`-o` 文件内容不变
- `--sort` 移除 `speen` 拼写兼容（仅接受 `speed`），帮助文本与 README 同步去掉 "speen is accepted as speed" 说明；误用 `speen` 报 `invalid --sort key`
- 并行实现：scoped 线程池 + channel，`probe_parallel` 通用化；新增 2 个单测（并行报告与串行一致、tvs 并行逐条输出与分组正确）；`cargo test` 211 项全绿

---

## v0.4.57（2026-10-11）

### 变更
- `sys net -t info` 输出新增 **国家/地区（Country）** 行：从注册主体（registrant）vCard `adr` 的第 7 个组件（RFC 6350 country，缺省时取最后一个非空组件）提取；注册主体（Registrant + Org）此前已输出，保持不变
- 单测断言更新（country=US、缺失 registrant 时 country 为 None）；`cargo test` 209 项全绿

---

## v0.4.56（2026-10-11）

### 变更
- `sys net` 新增 **`-t tvs`**：继承 `-t tv` 全部功能，额外支持聚合型 JSON（如 `{"urls":[{"url":"...","name":"..."}, ...]}`）
  - **单仓**：`sites` 非空数组每元素含 `name` 的 tv JSON → 按 `--sort` 规则（count/name/speed，默认 count,name）统计排序，`# count= name= speed=` 信息行
  - **多仓**：只有 `urls` 数组 → 只统计节点数（`# count=N`），按节点数 **desc** 排序
  - 两种都不是 → `[FMT]` 不输出
  - 最终**分组输出**：单仓组（`--- 单仓 (N) ---`）在前、多仓组（`--- 多仓 (N) ---`）在后，屏幕与 `-o` 文件一致
- 顺带重构：删除已无调用者的 `cmd_content`（json/file 分发内联到 `cmd_net`）
- 新增 3 个单测（单仓+多仓混合分组与多仓降序、仅多仓时无单仓标题、两种格式都不是时报 FMT）；`cargo test` 209 项全绿

---

## v0.4.55（2026-10-11）

### 变更
- `sys net -t info` 输出扩展：除注册商 / 创建 / 到期 / 状态 / 域名服务器外，新增 **注册人（Registrant）/ 注册组织（Org）/ 邮箱（Email）/ 地址（Address）** 字段（从 RDAP vCard 的 `registrant` 实体提取 `fn` / `org` / `email` / `adr`；地址按 vCard 组件拼接）；注册商未返回这些字段时不显示对应行
- 新增 2 个单测（registrant 字段解析、缺失 registrant 实体时不 panic）；`cargo test` 206 项全绿

---

## v0.4.54（2026-10-11）

### 变更
- `sys net` 新增 **`-t info`**：通过 RDAP（`rdap.org` 引导，HTTP/JSON，复用现有 reqwest）查询 `-n DOMAIN` 的注册信息——注册商 / 创建时间 / 到期时间 / 状态 / 域名服务器；**查不到该域名的注册数据时逐层回退到上层域名**（`a.b.example.com` → `b.example.com` → `example.com`），打印标注 `no registration data, using parent ...`；全链查不到打印 `no registration info found (checked: ...)`；`-n` 可重复
- `-i` 改为可选（info 模式用 `-n`，其他模式仍必填，缺省报错提示）；`-t info` 不需要 `-i`，`-o` / `-x` / `--sort` 忽略并 warning
- 新增 5 个单测（parent_domain 剥离、RDAP 字段解析、回退到父域、链耗尽无数据、多域名直查——本地 mock RDAP 服务器）；`cargo test` 204 项全绿

---

## v0.4.53（2026-10-11）

### 变更
- `sys net -t test` 末尾汇总补充**可访问（通过）地址总数**：`--- checked N address(es) in M line(s): K reachable, ... ---`（K = N - 不可达 - 排除 - 重复）；tv / file 模式末尾已有 `--- N of M URLs passed ---` 汇总（含通过总数），domain 已有 `--- N of M domains reachable ---`；`cargo test` 199 项全绿

---

## v0.4.52（2026-10-11）

### 变更
- `sys net -t tv` 有效性判定再放宽：**去除 `.lives` 节点必须存在的要求**（`lives` / `parses` / `rules` 全部可选）；仅要求 `sites` 为非空数组且每个元素含 `name` 字段；`[FMT]` 提示同步改为 `body lacks tv nodes (sites with name)`；`cargo test` 199 项全绿

---

## v0.4.51（2026-10-11）

### 变更
- `sys net -t tv` 有效性判定进一步放宽：**去除 `.rules` 节点必须存在的要求**（`rules` 变为可选，`parses` 同为可选）；仍要求 `sites` 非空数组且每元素含 `name`、顶层含 `lives`；`[FMT]` 提示同步改为 `body lacks tv nodes (sites with name, lives)`；`cargo test` 199 项全绿

---

## v0.4.50（2026-10-11）

### 变更
- `sys net -t tv` 有效性判定放宽：**去除 `.parses` 节点必须存在的要求**（`parses` 变为可选，如饭太硬.json 无 parses 也判有效）；仍要求 `sites` 非空数组且每元素含 `name`、顶层含 `lives` / `rules`；`[FMT]` 提示同步改为 `body lacks tv nodes (sites with name, lives, rules)`；单测补充 parses 可选与 rules 必需；`cargo test` 199 项全绿

---

## v0.4.49（2026-10-11）

### 变更
- `sys net -t tv`（旧名 `-t json`）有效性判定收紧：响应体除能解析为 JSON（**宽容**：先严格解析、失败则清理尾随逗号后重试，容忍真实数据常见瑕疵）外，还须满足 **tv 节点齐全**——`sites` 为非空数组且每个元素含 `name` 字段，顶层含 `lives` / `parses` / `rules` 节点；普通 JSON 或节点缺失判 `[FMT] body lacks tv nodes (sites with name, lives, parses, rules)`，无法解析判 `[FMT] body is not valid JSON`
- 新增 5 个单测（tv 节点判定、尾随逗号清理、宽容解析、端到端尾随逗号通过、非 tv JSON 拒绝）；既有 sort fixture 补齐 lives/parses/rules 与 name；`cargo test` 199 项全绿

---

## v0.4.48（2026-10-11）

### 变更
- `sys net -t tv`（旧名 `-t json`）`--sort` 参数不提供时的默认排序由 `name` 改为 **`count, name`**（先按 `.sites` 节点总数 desc，再按 `.sites.name` 含 `4k` 数量 desc）；help / README 同步更新；`cargo test` 194 项全绿

---

## v0.4.47（2026-10-11）

### 变更
- `sys net -t json` 改名为 **`-t tv`**（`-t tv` 使用，旧名 `json` 作为别名仍兼容，帮助文本与文档统一为 tv）；`--sort` / `-o` / warning 文本同步更新；`cargo test` 194 项全绿

---

## v0.4.46（2026-10-11）

### 变更
- `sys net -t json` 新增 `--sort` 统计排序输出（预期 JSON 形如附件：顶层 `sites` 数组 + `spider` 字段）：
  - `--sort count`：统计 `.sites` 节点总数，按数量 **desc** 排序
  - `--sort name`：统计 `.sites.name` 含 `4k`（大小写不敏感）的数量，按数量 **desc** 排序
  - `--sort speed`（拼写 `speen` 也接受）：下载 `.spider` 中的 url（字符串 `url;md5;...` 取 `;` 前部分，数组取首个字符串元素），按下载速度 **asc** 排序
  - `--sort` 可多个（`--sort count name`），按出现顺序多级排序；**不提供默认按 name 排序**
  - 每个通过的地址打印一行 url，下一行打印 `# count=<N> name=<N> speed=<速度或失败原因>` 测试信息；**屏幕与 `-o` 文件格式一致**（url + `#` 行）；不可达 / 非 JSON 地址照旧打印 `[NO]` / `[FMT]` 且不写入 `-o`
  - json 模式检测阶段不再逐条打印 `[OK]` 行（改为排序后输出）；file 模式行为不变
- 新增 6 个单测（统计计算、spider url 提取、--sort 解析别名/非法键、count desc、默认 name、多级排序、speed asc）；`cargo test` 194 项全绿

---

## v0.4.45（2026-10-10）

### 变更
- `sys net` 所有模式（domain / json / file / test）新增**重复检测**：本次运行内已检测地址缓存（`dup_key` 用 url crate 规范化——host 小写、默认端口归一，`HTTP://EXAMPLE.com/a` 与 `http://example.com/a` 视为同一地址）；再次出现的地址**不重复检测、直接删除**——test 模式走删除管线（单地址行删整行 / 多地址行只删该地址），json / file 不写入 `-o`，domain 不参与探测与排序；打印 `[DUP] <url> duplicate (already checked), skipped`，test / json / file 汇总行存在重复时追加 `, N duplicate`
- 新增 4 个单测（dup_key 规范化、test 重复行删除、同行重复地址只留一份、json 重复不写 -o）；`cargo test` 188 项全绿

---

## v0.4.44（2026-10-10）

### 变更
- `sys net -t test` 新增 `-x SUFFIX` 参数（可重复，如 `-x m3u -x zip`）：文件名后缀匹配（大小写不敏感、前导点可省略）的地址**不发起检测、直接删除**——例如 `-x m3u` 时 `https://www.main.com/123.m3u` 直接删除；单地址行整体删除，多地址行只删该地址，打印 `[X] <url> excluded (-x m3u)`，汇总行在存在排除时追加 `, N excluded`
- `-x` 仅在 `-t test` 模式生效；domain / json / file 模式传 `-x` 忽略并打印 warning
- 新增 4 个单测（后缀提取、可达地址带排除后缀不探测即删、多后缀 + 大小写不敏感、非 test 模式忽略）；`cargo test` 184 项全绿

---

## v0.4.43（2026-10-10）

### 修复
- `sys net` 错误信息截断 panic：`short_error` 原先按字节 `truncate(97)` 截断错误消息，当消息含多字节 UTF-8 字符（如 Windows 中文错误信息“由于目标计算机积极拒绝，无法连接。 (os error 10061)”）且超过长度时，截断点落在字符中间，触发 `assertion failed: self.is_char_boundary(new_len)` 崩溃；改为 `truncate_ellipsis` 按 UTF-8 字符边界安全截断（保留 “…” 后缀预算），并新增多字节 / 混合字符截断单测
- 本地 TCP 测试服务器改为先读请求头再响应（模拟真实服务器行为），消除偶发的 `received unexpected message from connection` 竞态；连续 3 次 `cargo test` 均 180 项全绿

---

## v0.4.42（2026-10-10）

### 变更
- `sys net -t test`：**每处理一行打印一行检测结果**，并带进度计数 `[当前行/总行数]`（如 `[3/6]`，总行数含空行与注释行）：
  - 逐地址显示 `[OK] <url> time=...` 或 `[NO] <url> error=<原因>`，行尾标注 `-> kept` / `-> line deleted` / `-> kept (bad address removed)`；空行 / `#` 注释行 / 无地址行显示 `[skip]`
  - 原有的删除与 `-o` 清理输出行为不变；`cargo test` 179 项全绿

---

## v0.4.41（2026-10-10）

### 新增
- `sys net` 新增 `-t test` 检测模式：只探测地址**可访问性**（收到任何 HTTP 响应即视为可访问，与 json/file 口径一致）：
  - 不可访问的地址在屏幕打印 `[NO] <url>  error=<原因>`，并从源文件删除
  - 单地址行不可访问 → 删除整行；多地址行只删除其中不可访问的地址、保留可访问部分，整行全部不可访问才删除整行；空行、`#` 注释行、不含地址的文本行一律保留
  - `-o FILE` 把删除后的源文件结果（保留的行按原顺序、原文本）写入指定文件，一行一个
  - 重构地址提取为带行内位置的 `UrlSpan`，`rebuild_line` 按 token 重建行
  - 新增 4 个单测（坏行整行删除、多地址行保留可访问部分、全坏行删除、周围文本保留），`cargo test` 179 项全绿

---

## v0.4.40（2026-10-10）

### 新增
- `sys net`：新增**网络探测**子命令，从文件批量读取 http(s) 地址并逐条探测：
  - `-i FILE`：逐行读取输入文件；以 `#` 开头（前导空白后判断）的行跳过；每行可含**多个地址**——地址从 `http`（大小写不敏感）起，截取到下一个空白字符为止并修剪行尾标点（如 `http://a.com/x,` → `http://a.com/x`），scheme 统一为小写并校验 host
  - `-t domain`（默认）：对每个地址发起**连续 3 次** GET，取平均响应时间；全部探测完毕后按平均响应时间**升序**打印，不可达地址列在末尾并附原因
  - `-t json`：响应体必须可按 **JSON** 解析才通过；`-t file`：响应体必须为**普通文本**（UTF-8 非二进制且非 JSON）才通过
  - json / file 模式**每测完一个立即打印** URL / 是否可用 / 状态码 / 响应时间 / 实际格式（json / text / binary）
  - `-o FILE`：把检测成功（可访问且格式符合 `-t` 要求）的 url 写入指定文件，一行一个（仅 json / file 模式有效）；`--timeout SECONDS` 单次请求超时（默认 10 秒）
  - 新增 18 个单元测试（含本地 TCP 服务端端到端探测：JSON / text / binary 格式判定、3 次取平均、按平均排序、`-o` 过滤写入、注释行跳过、一行多地址提取、无效超时拒绝），`cargo test` 175 项全绿

---

## v0.2.9（2026-10-04）

### 新增
- `sys ai cn-model`：新增**国内 AI 模型查询**子命令，数据源为 [datalearner](https://www.datalearner.com/ai-models/pretrained-models) 的预训练模型列表（约 1015 个模型，分页抓取 + slug 去重，24h 本地缓存 `datalearner-models.json`）：
  - 只做查询：按名称精确查询 / `-s` 子串搜索（别名也匹配）；**没有** `--list`、**没有** `--open`（该数据源不提供开放权重信息）
  - `--date YYYY-MM-DD`：以卡片 **published 发布日期**为筛选标准，只显示严格晚于该日期的模型；单独使用 `--date` 时直接列出全部晚于该日期的模型
  - 字段：id（slug）/ name / provider / aliases / type 徽章 / category / published / url；`--json` / `-o json` 输出 JSON 数组，`-o csv` 输出 8 列 CSV；`--refresh` 强制重抓
- `sys ai chat`：新增**聊天**子命令，按 OpenAI `/chat/completions`（type=openai，默认）或 Anthropic Messages API（type=anthropic）标准发请求：
  - 配置文件默认 `~/.sysenv/config.yaml`（缺失明确提示；`-c/--config` 覆盖）；`clients` 存放多 Provider（必填 name / api_base / api_key / models，weight 缺省 1，max_tokens 可选）
  - 顶层 `model` 选择规则：缺失 → 第一个 Provider 的第 1 个模型；`provider:model` → 双匹配；`provider:*` → 该 Provider 全部模型**加权轮询**；裸 `model` → 跨 Provider 匹配模型合集**加权轮询**（轮询状态持久化于配置同目录 `chat_state.json`）
  - 顶层 `stream: true` 默认 SSE 流式逐字输出；`--no-stream` 关闭；`--debug` 时自动非流式
  - `--debug`：打印实际 HTTP 请求（方法 / URL / 头 / 体）与响应（状态 / 头 / 体）到 stderr
- `sys ai task`：新增**任务模板聊天**子命令：
  - 无参数列出 `tasks` 的 name / desc（最多 10 个）；`-t <name>` 取对应任务组装消息后走 chat 通道
  - `msg` 占位符 `{key:默认值}`：命令行传 `key:值` / `key=值` 则替换，否则用默认值（如 `{country:深圳}` + `country:北京` → 北京）
  - `msg` 前缀 `file://`（读本地相对路径文件）与 `url:`（抓取网络内容）
- `sys http --help`：新增帮助参数，打印接口参数说明与示例（http 子命令 `-h` 仍是 httpie 语义的“只打印响应头”）
- `sys http --debug`：新增调试参数，把实际 HTTP 请求（方法 / URL / 请求头 / 请求体，含 Content-Length）与响应（状态 / 响应头 / 响应体）打印到 stderr，stdout 保持正常输出
- 新增脱敏示例配置 `doc/config.example.yaml`（README 引用的 `doc/config.yaml` 为本地测试用真实配置，文档一律不出现真实密钥）

---

### 新增
- `sys task`（`stask`）新增 `-o/--port <PORT>`：查看**占用指定端口的进程**（与 `list` 的名称 / PID 过滤可组合；`kill` 模式不适用）；Windows 用 `netstat -ano`，Linux 用 `ss -ltnp`

---

## v0.2.7（2026-10-04）

### 新增
- `sys task`（`stask`）不加参数时默认列出**全部进程**（等价于 `sys task list`），与 `sai model` 无参默认全量的行为保持一致

---

## v0.2.6（2026-10-04）

### 变更
- `sys ai model` 文本列表（无参 / `--list` / `--search` / 多匹配）默认展示改为**带列名对齐表格**：`ID  NAME  FAMILY  LAST UPDATED`（原先为无表头的 `id 名称 provider` 制表符行）；单模型详情视图（`== 标题 ==` + 字段行）不变

---

## v0.2.5（2026-10-04）

### 新增
- `sys ai model --date YYYY-MM-DD`：由 v0.2.4 的 `--data` 更名定稿（功能不变：仅显示 `last_updated` 晚于该日期的模型）
- `sys ai model --open`：仅显示 `open_weights: yes` 的模型；可与 `--date` / 无参全量 / `--list` / `--search` / 名称查询组合（先按日期、再按开放权重过滤）
- 新增 `sys task` 子命令（Windows / Ubuntu）：
  - `task list [NAME]`：列出进程的 PID / 名称 / 可执行文件路径；NAME 按名称模糊匹配（子串、大小写不敏感），传数字则按 PID 精确查询
  - `task kill <PID|名称>`：按 PID 或名称终止进程；名称模糊匹配会终止全部命中进程；`-f/--force` 强制终止（Linux 发送 SIGKILL，默认 SIGTERM）；无权限等失败项单独提示
- 快捷垫片新增 `stask`（`sys task`），`sys short` 一次性安装 6 个短命令

---

## v0.2.4（2026-10-04）

### 新增
- `sys ai model`（`sai model`）不加参数时默认列出**全部模型**（原为报错提示需提供名称或 `--search/--list`）；`--limit N` 仍可限制条数，`--list` 保持默认 20 条分页
- `sys ai model --data YYYY-MM-DD`：仅显示 `last_updated` **晚于**该日期（严格大于）的模型；可与 `--list` / 无参全量 / `--search` / 名称查询组合（先按日期过滤再匹配）；日期格式非法时给出明确报错

---

## v0.2.3（2026-10-03）

### 新增
- `-V` / `--version` 版本信息增加署名：`sys 0.2.3 (Made by Gary-china)`（版本号自动跟随 Cargo.toml）

---

## v0.2.2（2026-10-03）

### 修复
- `sys ai model` 详情输出（单命中 / canonical 优选）缺少标题：现在顶部增加标题行 `== <模型标题> ==`，取自 models.dev 页面上的模型标题（即模型 `name` 字段，缺失时回退为 `id`），例如 `sai model deepseek-chat` 首行输出 `== DeepSeek V3/Deepseek Chat ==`
- 多匹配列表 / 搜索 / 列表模式不受影响（每行已含标题列 `name`）

---

## v0.2.1（2026-10-03）

### 新增
- `sys ai model` 与 `sys ai provider` 增加 `-o/--output-format json|csv` 可选参数：
  - `-o json`：输出 **JSON 数组**格式（与 `--json` 等价，单命中也是数组；适用于详情 / 搜索 / 列表 / 多匹配全部场景）
  - `-o csv`：输出带表头的 CSV 表格（RFC-4180 转义）——model 24 列、provider 6 列
- 非法格式值（如 `-o xml`）由 clap 校验拒绝并提示可选值 `json, csv`；`--json` 与 `-o` 互斥

### 发布规范
- release 产物统一使用 **UPX 压缩**（实测：Windows 5.1 MB → 1.79 MB，Linux 5.1 MB → 1.91 MB）
- 产物文件名包含平台 + 架构 + 版本号：
  - `sys-windows-x86_64_v0.2.1.exe`
  - `sys-linux-x86_64_v0.2.1`
- 新增本文件（`patch.md`），记录每次版本号变更的新增 / 修复内容

---

## v0.2.0（2026-10-03）

### 新增
- `sys ai` 子命令：查询 models.dev（`https://models.dev/api.json`，226 个 Provider、8000+ 模型）
  - `ai model NAME`：按模型名称 / id / canonical id 精确查询；多 Provider 同名时自动优选 canonical 条目（如 `gpt-4.1` → OpenAI）并提示其余 Provider 数量
  - `ai provider NAME`：按 Provider id / 名称查询（api / npm / 模型清单）
  - `-s/--search` 子串搜索（grep 风格列表）、`--list` 分页浏览、`--limit` 条数控制（默认 20）、`--json` 原始输出、`--refresh` 强制重抓
  - 24 小时本地缓存（Windows `%LOCALAPPDATA%\sys\`，Linux `$XDG_CACHE_HOME` 或 `~/.cache/sys/`）
- `sys short` 子命令：一键安装全部子命令的快捷垫片 `spath` / `senv` / `slink` / `shttp` / `sai`（Windows `.cmd`、Linux 可执行 sh 脚本），自动持久化加入 PATH，支持 `--dir` / `--force` / `--temporary`

### 修复
- 修正 ai 查询匹配逻辑：`--search` 改为列表输出；裸名称多 Provider 匹配时优先 canonical 条目

---

## v0.1.0（2026-10-03）

### 新增（初始版本）
- `sys path`：PATH 增删查改（`list` / `add` / `remove` / `has`），自动持久化并同时作用于当前环境；`-p` 前置、`--scope user|machine` 双作用域、`--temporary` 临时模式；自动转绝对路径、去重
- `sys path export/import`：PATH 与 Windows 注册表文件（`.reg`）、JSON、文本双向导入导出，支持合并与 `--replace` 整体替换
- `sys link`：把任意文件链接进 PATH 目录（硬链接 → 符号链接 → 拷贝自动回退；Windows 自动生成 `.cmd` 垫片；`--name` / `--method` / `--system` / `--dir`）
- `sys env`：环境变量 `get` / `set` / `unset` / `list`，默认持久化，`--temporary` 仅当前 shell，`--scope machine`
- `sys http`：httpie 兼容 HTTP 客户端（`-j/-f/--multipart/--raw`、`-p/-h/-b/-m/-v/-o/-d/-q`、`-a/-A` 认证、`-F/--follow/--max-redirects/--timeout/--proxy/--check-status/--offline/--verify/-I/--default-scheme`；请求项语法：`key=value`、`key:=json`、`key==value`、`key:value`、`key@file`、`@file`、嵌套 JSON）
