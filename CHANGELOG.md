# sysenv 更新日志（CHANGELOG）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sysenv-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

## v0.4.12（2026-10-06）

### 新增
- **`sysenv ai image`（`sai image`）**：通过 Provider 的 OpenAI 兼容 **Images API（`POST {api_base}/images/generations`）** 生成图片
  - 与 `ai chat` 共用 `~/.sysenv/config.yaml` 的 `clients` 配置与模型选择规则：`-m/--model` 支持 `{provider}:{model}` / `{model}` / 逗号分隔多选，多模型目标按 `weight` 加权轮询，失败自动降权并尝试下一个模型（成功替补升权，写回配置文件）
  - `-o/--output DIR` 指定保存目录（默认当前目录）；`-n/--count N` 数量（默认 1）；`-s/--size SIZE` 尺寸（默认 `1024x1024`，原样传给 API）；`--url` 改为请求图片 URL 并下载保存（默认请求 `b64_json` 本地解码保存）
  - 图片保存为 `sysenv-ai-image-<时间戳>-<序号>.<格式>`，扩展名按文件头魔数识别（png / jpg / gif / webp），文件名冲突自动加 `-1`、`-2` 后缀；每张图片的保存路径打印到 stdout
  - 仅支持 OpenAI 兼容 Provider（`type: openai`）；`type: anthropic` 目标在发请求前明确报错；`--list-provider / --list-model` 与 `ai chat` 一致
  - 冒烟验证：请求正确到达智谱 `/images/generations`（429 余额不足系账号余额问题，接口与错误透传正常）；agnes 对话模型返回 400 并提示"Use /v1/chat/completions"
- **`sysenv search`（`ssearch`）**：接入**博查 AI 网页搜索 API**（`POST https://api.bochaai.com/v1/web-search`）提供联网搜索
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
- `sysenv ai task --list-source`：列出全部内置搜索源的**名称、类型（api 直连数据接口 / search AI 聚合搜索 Sogou 优先 Bing 兜底 / generic 通用搜索需 q/query 参数）、用途与访问地址**，共 19 个源（zhihu/baidu/bilibili/github/hn/toutiao/tophub/oschina/smzdm/bing/sogou/dxtower/enlightent/cls/dongchedi/autohome/szhousing/penalty/company）

## v0.4.7（2026-10-05）

### 新增
- `sysenv ai task` 新增两个搜索源：
  - `penalty`：查询**某人/某单位的行政处罚、失信被执行人信息**（公开公示渠道，AI 聚合搜索）
  - `company`：查询**公司工商注册信息**（法定代表人、注册资本、成立日期等，公开公示渠道，AI 聚合搜索）
- 实现说明：实测信用中国（creditchina.gov.cn）无响应、国家企业信用信息公示系统（gsxt.gov.cn）521、中国执行信息公开网/裁判文书网/爱企查均需验证码或 JS 渲染、无法程序直连 → 采用 AI 聚合搜索（Sogou 优先、Bing 兜底）抓取公开公示页面（企查查/爱企查/百科/政府公示等）回填，模型如实总结；查询对象由模型的 `name` 参数指定，无相关记录时模型如实说明
- **`-t` 手动指定任务时自由文本参数自动填充任务的第一个声明参数**（如 `-t company 字节跳动` → `name=字节跳动`，msg 中 `{name}` 占位符与工具 query 均被替换），`weather` 等既有任务不受影响
- 配置示例新增 `penalty` / `company` 任务（`~/.sysenv/config.yaml` 与 `doc/config.yaml`）

## v0.4.6（2026-10-05）

### 新增
- `sysenv ai task` 新增搜索源 `szhousing`：获取**深圳房源销售情况**（新房/二手房成交套数等，深圳房地产信息平台公开数据）
- 实现说明：`fdc.zjj.sz.gov.cn` 经实测部署瑞数动态 WAF（curl 全量浏览器头仍返回 HTTP 412 验证页），无法程序直连；改用 AI 聚合搜索（Sogou 优先、Bing 兜底）抓取公开渠道（乐有家/中原/住建局官网等）的成交数据报道回填，模型如实总结并注明来源
- 配置示例新增任务 `szhousing`（`~/.sysenv/config.yaml` 与 `doc/config.yaml`）

## v0.4.5（2026-10-05）

### 新增
- `sysenv ai task` 新增 **10 个内置搜索源**，覆盖用户点名的 5 组数据源：
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
- `sysenv ai task` **任务工具执行改为纯代码 HTTP 实现，禁止本地 shell 命令**（移除 `tool:` 命令模板，删除 `run_tool_cmd`）：任务可声明两种工具，function_call 选中后由程序执行并**回填真实数据**给模型回答：
  - **`api: <URL模板>`** —— 固定 HTTP 接口：`{key}` / `{key:默认值}` 占位符由模型 tool 参数填充（空值回退默认值），响应体原样作为工具结果；`params: [k1, k2]` 声明可选参数并加入函数 schema
  - **`search: <源名>`** —— AI 模糊联网搜索：内置 `zhihu`（知乎日报热门）、`baidu`（百度实时热搜）、`bilibili`（B 站热门）、`github`（近 7 天新建星榜，可传 `date` 参数）、`hn`（Hacker News）五个搜索源，程序抓取头条列表回填，模型据此筛选总结
- 自研 YAML 解析器支持内联数组 `[a, b]`（`params: [lat, lon]` 与块列表写法均可）
- 配置示例新增 `topnews`（zhihu）与 `topshow`（baidu）任务，weather 改用 `api:` 写法

### 修复
- 模型 function_call 参数传空字符串时，URL 模板占位符回退使用默认值（此前空值会生成无效 URL）

## v0.4.3（2026-10-05）

### 新增
- `sysenv ai task` 无 `-t` 且带用户请求（参数或 stdin）时改为 **function_call 自动路由**：
  - 每个任务的 `name` 作为函数名、`desc` 作为函数描述注册为 tools（OpenAI / Anthropic 双协议）
  - 大模型用 function_call 选择最匹配的任务，用户请求通过 `input` 参数传入
  - 自动执行选中的任务：把任务 `desc` + 用户请求组装成消息发给模型，输出最终回复
  - 模型未选择任何任务时明确报错，提示改用 `-t NAME` 手动指定
- 兼容与保留：无输入仍列出任务；`-t NAME`（模板占位符替换）与 `-t *`（0-10 分打分匹配）行为不变；路由请求同样走动态权重轮询与失败自动切换

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
