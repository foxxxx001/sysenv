# sys 更新日志（Patch Notes）

版本号变更记录：新增功能 / 修复 / 发布规范。产物命名规范：`sys-<平台>-<架构>_v<版本>`，release 产物使用 UPX 压缩（见 README「构建与发布」）。

## v0.6.1（2026-10-19）

### 变更
- **csv / table 表头判定规则修正**：
  - 第一行**默认即列名**（不再按"首行全数字"自动推断 colN）
  - 第二行为 `----` 分隔线（如 ps 输出的 `------- ------`）时识别为表头下划线并**跳过**，不再成为垃圾数据行
  - `--no-header` 显式启用 `col1..colN`（唯一途径）；`--header` 强制首行为列名
- 移除 `table_cell_is_numeric` 自动推断函数；帮助文本同步

### 测试
- `cargo test --bin sys` 319 -> 321 项全绿（新增 table/csv 分隔线跳过各 1 项；原"全数字首行自动 colN"断言改为默认列名 + 显式 --no-header）

---

## v0.6.0（2026-10-19）

### 变更
- **CSV 与 table 列标题统一虚拟为键名**：`csv_to_value` 增加 `header: Option<bool>`，与 table 相同语义——`None`（自动）首行含任一非数字单元格即作表头（列标题为键名），全数字首行视为数据、自动生成 `col1..colN`；`--header` / `--no-header` 对 `-i csv` 同样生效（此前仅 table）；空表头单元格回退 `colN`、空行跳过
- 有表头的 CSV 行为不变；示例：`... | sys data -i csv '[*].filter(col1==4)'`

### 测试
- `cargo test --bin sys` 315 -> 319 项全绿（新增 csv 表头判定 4 项）

---

## v0.5.13（2026-10-19）

### 新增
- **根数组函数表达式**：`resolve_single` 增加根数组特例——路径为裸 `[*]` 且根是数组时，寻址数组本身而非展开全部元素。`sys data -i table '[*].filter(Id==7360)'` / `'[*].search(...)'` 可直接对整张表过滤 / 搜索（此前报 `matched N values (expected exactly one)`）
- 示例：`ps | sys data -i table '[*].filter(Id==7360)'`——PowerShell 表格化输出中的 `------` 分隔线行（Id 为字符串 `--`）会被谓词自然排除

### 变更
- 版本 0.5.12 -> 0.5.13；README（中 / 英）data 特性行补充根数组 `[*].filter(...)` 说明
- `cargo test --bin sys` 314 -> 315 项全绿（新增 `filter_on_root_array_via_wildcard`）

---

## v0.5.12（2026-10-19）

### 新增
- **`sys data -i table`（空格分隔表格解析，如 ps 输出）**：
  - `ps aux | sys data -i table` 把每行解析为一条数据，列间以连续空白分隔
  - **列标题自动判定**：首非空行存在任一非数字单元格即视为表头（可直接用列名查询，如 `[0].PID` / `[*].COMMAND`）；首行全数字则视为数据行，自动生成 `col1..colN` 列标题；`--header` / `--no-header` 可强制指定（两者互斥，clap 校验）
  - **溢出/缺失容错**：行内多于表头的列以单空格并入最后一列（ps COMMAND 含空格场景）；缺列补空串；空行跳过；CRLF 兼容；单元格类型推断复用 CSV 规则（数字 → 数值、true/false → 布尔、空 → 空串）
- **`-o table`（表格输出）**：对象数组 / 单对象 / 纯标量数组均可输出，列宽 = 表头与单元格最大字符数，单空格对齐；含换行单元格 JSON 转义；对象键按首现序
- **根数组选择器**：`parse_selector` 支持行首 `[`（`[0]` / `[-1]` / `[*]`），`sys data -i table '[0].PID'` 可直接按列标题寻址根数组元素
- **`-o table` 标量读取**：`render_get` 对 Table 格式下数字 / 布尔 / 字符串标量直接打印（字符串不带引号），不再走表格序列化器报错

### 变更
- 版本 0.5.11 -> 0.5.12；`-i` / `-o` 帮助补充 table 格式与 `--header` / `--no-header` 说明；README（中 / 英）data 特性行更新为六格式 + table 解析 + 根数组查询示例
- `cargo test --bin sys` 300 -> 314 项全绿

---

## v0.5.11（2026-10-17）

### 移除
- **`sys search` 子命令整体删除**（联网搜索，博查 Bocha AI Web/AI Search API）。删除范围：
  - main.rs：`Cmd::Search` 变体、`SearchArgs` 结构（query / freshness / summary / count / page / include-domains / exclude-domains / ai / no-answer / config / json / debug）、分发分支、`run_search`、long_about 与 Examples 中的 search 引用、short 帮助文本中的 ssearch
  - chat.rs：`Config.search` 字段与 `SearchKey` 结构、配置解析中 `search:` 段读取、Bocha 全部代码（`BOCHA_API` / `BOCHA_AI_API` 常量、`bocha_request` / `bocha_body_common` / `bocha_search_value` / `bocha_ai_search_value` / `bocha_result_items` / `bocha_ai_answer` / `bocha_result_rows` / `bocha_result_text` / `bocha_search` / `bocha_ai_search`）、`cmd_search`；测试移除 8 项（`config_parses_search_section` + 7 项 bocha_*）；2 处测试 Config 构造同步去 `search` 字段
  - short.rs：`ssearch` 垫片移除（SHIMS 8 → 7：spath/senv/slink/shttp/sai/stask/sfile）
- 本机已安装的 `ssearch.cmd` 垫片不主动清理（按发布约定不更新 short），再次执行 `sys short` 后自动消失

### 测试
- `cargo test --bin sys` 308 -> 300 项全绿（移除 8 项 search 测试；short 断言更新为 7 项垫片）

---

## v0.5.10（2026-10-17）

### 移除
- **`sys con` 子命令整体删除**：格式转换功能由 `sys data`（别名 `dasel`）完全覆盖（`data` 支持 JSON / YAML / TOML / CSV / MD 五格式互转，`-i` / `-o` / `-f` / `-w` / `--root` / `--compact`，比 `con` 的 json / csv / md / yaml 多 TOML）。删除范围：
  - main.rs：`Cmd::Con` 变体、`ConArgs`、分发分支与帮助文本中的 scon 引用
  - convert.rs：con 专属代码（`Format` 枚举、`detect_format`、`parse`、`parse_yaml_value`、`parse_args`、`CON_HELP`、`cmd_con`）与 4 项专属测试（`format_parse_and_ext` / `json_yaml_roundtrip` / `detect_common_formats` / `yaml_tab_indentation_fallback`）删除；**保留** dasel 共享的解析 / 序列化函数（`csv_to_value` / `md_to_value` / `yval_to_json` / `value_to_csv` / `value_to_md` 及内部辅助），模块文档改写为「共享结构化数据转换辅助」
  - short.rs：`scon` 垫片移除（SHIMS 9 → 8：spath/senv/slink/shttp/sai/ssearch/stask/sfile）
  - 注释同步：main.rs / file.rs 中「like `con`'s -file/-out」→「like `data`'s raw tokens」；dasel.rs 注释标注 con 已删除
- 本机已安装的 `scon.cmd` 垫片不主动清理（按发布约定不更新 short），再次执行 `sys short` 后自动消失

### 测试
- `cargo test --bin sys` 312 -> 308 项全绿（移除 4 项 con 测试；short 断言更新为 8 项垫片）

---

## v0.5.9（2026-10-17）

### 新增
- **`sys log`（彩色 less 式日志查看器）**：`sys log --file app.log [--level LEVEL] [--search KW] [--line N] [--newer TIME] [-f]`。解析常见日志并按语义着色，交互同 less——PageUp / PageDown 翻页、↑/↓ 逐行、Home / End / g / G 跳转、q / Esc / Ctrl-C 退出、n / u 在搜索命中间跳转（循环、居中）；底部状态栏：` 文件名  L首-末/总数  level=..  "kw" i/N  following...  q=quit PgUp/PgDn=page ↑↓=line n/u=search`
- **时间解析（行首多种常见格式，解析后橙色 RGB(255,165,0) 显示）**：ISO 短横线 `2024-01-15 10:23:45`、ISO-T/Z `2024-01-15T10:23:45.123Z`（毫秒可选）、Log4j 逗号毫秒 `2024-01-15 10:23:45,123`；斜杠 `2024/01/15 10:23:46`；nginx access log `15/Jan/2024:10:23:49 +0800`——时间在行中间也识别（行内前 80 字节扫描），前缀 `127.0.0.1 - - [` 完整保留（entry() 对 `tm.start > 0` 增加前缀 span，断言 joined == line）；syslog `Jan 15 10:23:48`（无年份取当年，落在未来 1 天以上自动回退上年）；US `01/15/2024`；带时区时间按墙钟解释（`off.from_local_datetime`，`10:23:45 +0800` 显示为 `10:23:45+08:00` 而非错误换算成 UTC 的 `18:23:45`）
- **级别渐变色**：trace 浅绿(178,230,178) → debug 绿(102,204,102) → info 绿(0,180,0)——info 及以下绿色、越低越浅；warning 红(255,90,90) → error 深红(204,0,0) → fatal 最深红(140,0,0) 加粗。别名：warn==warning、err==error、critical/crit/alert/emerg==fatal；`--level` 精确过滤（无级别行剔除）
- **app 标志（深黄色 RGB(184,134,11)）**：时间后形如 `[http-nio-8080-exec-1]`、`app[1234]`、`[01-node-5]` 的标记
- **`--file FILE`**：指定日志文件（缺省读 stdin，`-` 亦为 stdin）；**`--search KW`**：大小写不敏感，显示行号（灰色 RGB(130,130,130)）、命中反显；**`--line N`** 从第 N 行后开始（保留原始行号）；**`--newer TIME`** 只显示 ≥ 该时间的日志（无时间戳延续行保留）；**`-f`** 追踪文件（150ms 轮询增量 + 截断/轮转重读，仅文件可用，stdin 报错）
- 非 TTY（管道 / 重定向）输出纯文本无 ANSI（安全）；TTY 进 crossterm 全屏交互；新依赖 crossterm 0.28

### 修复
- `DateTime::from_utc` 弃用告警 → 改用 `from_local_datetime` / `from_utc_datetime`；`Span.reverse` 字段从未被读，删除消告警
- nginx 行内时间解析曾丢弃时间之前的前缀文本，已修复并加断言（见上）

### 测试
- 新增 29 项 `log::tests` 单测（时间格式 / 时区语义 / 级别别名 / app 提取 / 过滤器 / 搜索命中列表与行号 / 分页滚动 clamp / -f 增量与截断重读 / 错误路径）；`cargo test --bin sys` 312 项全绿

### 冒烟验证（真实二进制，Windows）
- `sys log --file dev/sample.log` 纯文本 8 行完全复原（nginx 前缀修复后）；`--level warn` 只留 WARN；`--search app` 8 行带行号；`--newer "2024-01-15 10:23:47"`、`--line 6 --search GET` 正确；`--level bogus` / `--newer garbage` / stdin `-f` 报友好错误退出码 1
- 真实控制台启动 TTY 查看器：raw mode + 全屏渲染不崩溃（进程存活等待按键）；交互按键路径由单测覆盖

---

## v0.5.7（2026-10-17）

### 新增
- **`sys data`（别名 `dasel`）**：结构化数据查询 / 修改 / 转换模块（dasel v3 核心工作流的 Rust 实现）。支持 JSON / YAML / TOML / CSV / Markdown 五种格式互转与统一选择器：`a.b[0]` 点路径、`list[-1]` 负索引、`list[*]` 通配、`m["中文名"]` 引号键（含点号 / 空格 / CJK）；表达式 `PATH = VALUE`（JSON 字面量或纯文本，`= null` 删除）、`+= / -= / *= / /=` 数值与字符串追加；裸选择器输出原始标量（dasel 风格），容器按输出格式序列化；`--root` 输出修改后整文档、`--compact` 单行 JSON、`-w FILE` 修改写回原文件（stdout 仍打印所改值，配 `--root` 则静默）；`-f FILE` 读文件（后缀决定格式）、**`-u URL`** GET 拉取远端内容作输入（默认 json，`-i` 可覆盖，30s 超时，非 2xx 报错含状态码与响应片段）；`-i` 显式传入时优先于文件后缀 / URL 默认格式；无 `-f/-u` 时读 stdin（管道）。参数：`sys data [EXPR] [-f FILE | -u URL] [-i FMT] [-o FMT] [-w FILE] [--compact] [--root]`
- **tab 缩进 YAML 兼容**：`parse_yaml_tolerant` 先把每行前导 tab 归一化为 4 空格（引号标量内的 tab 保留），再走 serde_yaml 严格解析，失败回退项目自带容错块解析器（chat::parse_yaml，同样 tab 归一 + BOM 剥离）；检测器 `detect_format` 在 YAML 前置 `looks_like_yaml` 结构启发式（须存在 `key:` / `- item` 行），避免把 `a,b` 之类逗号文本误判为 YAML，CSV / MD 探测顺序前移
- **中文 round-trip 验证**：JSON↔YAML↔TOML↔CSV 全链路 CJK 不转义不乱码（serde_json 默认保留非 ASCII；serde_yaml 对含换行的中文自动用块标量）；新增 16 项 `dasel::tests` 单测（选择器解析 / 表达式切分含引号内 `=` 不误切 / tab YAML / 中文多行块标量 / TOML 往返 / 通配读取 / 运算符 / 格式优先级 -i 覆盖后缀与 URL 默认 json / 增删改查）
- 新依赖：toml 0.9、libc 0.2（stdin tty 检测，Windows 走 GetConsoleMode）

### 修复
- 选择器方括号内引号键 `m["中文名.字"]` 初版被当作数组下标解析而报错，已支持 `["…"]` / `['…']` 形式
- `detect_format` 对无结构纯文本（如两行 CSV 头）误判 YAML 导致类型全变字符串，改为先 CSV/MD、YAML 需通过结构启发式

### 测试
- `cargo test --bin sys dasel::` 16/16 全绿；分批回归：convert/file/hash 39、httpie/net 75、chat/ai 128、path/env/link/lock 1（合计 243 + dasel 16 = 259 项通过）——全量一次性跑超时会话超时限制，按模块过滤分批执行（dev/test-progress.md 记录进度）

---

## v0.5.6（2026-10-16）

### 修复
- **`sys file -n`（位置无关的行号输出）**：`-n` 放在模式后面时不再报 `path `-n` does not exist`（1 参 stdin 搜索），`sys file PATTERN PATH -n`（2 参目录搜索）不再被误判为替换模式。根因与 v0.5.5 的 `-l` 相同：`FileArgs.args` 的 `allow_hyphen_values` 会把第一个位置参数之后的旗标状 token 吞进 ARGS。修复：把 `split_line_count_args` 泛化为 `split_output_flag_args`，从 ARGS 一次扫描并剥离 `-l`/`--line-count` 与 `-n`/`--line-numbers`（标记任意位置均可，`-lfoo` / `--l` / `-nfoo` / `--n` 不误认），扫描结果写回 `FileOpts`，所有模式统一生效；1 参 stdin 搜索输出 `行号:内容`，2 参目录搜索输出 `路径:行号:内容`（本就带行号，`-n` 不再被吞导致报错/误入替换）
- 文档同步：src/file.rs 头部 Usage/Options、src/main.rs FileArgs 注释；版本 0.5.5 -> 0.5.6；`cargo test` 273 项全绿（`split_line_count_args_any_position` 扩展为 `split_output_flag_args_any_position`，新增 `-n`/`--line-numbers`/组合/不误认用例）

### 冒烟验证（真实二进制，Windows）
- `("动漫A","普通","动漫B") | sys file "动漫" -n` -> `1:"动漫A"` / `--` / `3:"动漫B"`，与 `sys file -n "动漫"` 前部写法完全一致
- `sys file 动漫 <含3行txt> -n` -> `路径:1:...` / `--` / `路径:3:...`，与无 `-n` 对照一致，且**文件内容未改动**（修复前 2 参尾部 `-n` 会进入替换模式）
- `-l`（匹配行数 2）、`-md5`、替换模式 `sys file hello hi <文件>` 回归正常

---

## v0.5.5（2026-10-16）

### 修复
- **`sys file PATTERN -l`（位置无关的匹配行统计）**：`-l` 跟在模式后面时不再报 `path `-l` does not exist`。根因：`FileArgs.args` 位置参数开了 `allow_hyphen_values`，clap 会把第一个位置参数之后的旗标状 token 吞进 ARGS，`-l` 因此未置位 `line_count` 而被当作路径/替换串处理；更严重的是 `sys file PATTERN -l PATH` 会被误判为替换模式，把文件中的 PATTERN 真的替换成字面量 `-l`（数据损坏风险）。修复：在 `cmd_file` 中仿照 `-md5`/`-sha` 的 `split_hash_args` 模式，新增 `split_line_count_args` 从 ARGS 重新扫描并剥离 `-l` / `--line-count`（标记可出现在任意位置，`-lfoo` / `--l` 之类不误认），`line_count` 以扫描结果为准；同时消除了 `FileOpts.line_count` 字段的 dead_code warning（以 `opts.line_count` 作为扫描基准）
- 文档同步：src/file.rs 头部 Usage/Options、src/main.rs FileArgs 注释；版本 0.5.4 -> 0.5.5；`cargo test` 272 -> 273 项全绿（新增 `split_line_count_args_any_position` 1 项）

### 冒烟验证（真实二进制，Windows）
- `Get-Content 单仓聚合.json | jq '.sites[].name' | sys file "动漫" -l` -> `33`，与独立统计（`Select-String '动漫'` 计数 33）一致；`-l` 前置写法 `sys file -l "动漫"` 同为 `33`
- `sys file 动漫 -l <含 2 行动漫的 json>` -> `path: 2` 且文件内容校验未改动（修复前会把 "动漫" 替换为 "-l"）；`sys file -md5 <文件>`、替换模式 `sys file hello hi <文件>` 回归正常
- `... | sys file -l`（stdin 总行数）-> `816`；`sys file -l <文件>` -> `12813`

---

## v0.5.4（2026-10-15）

### 新增
- **`sys file -md5` / `-sha`（哈希模式，优先级最高）**：`-md5 FILE` 对 FILE 做 MD5 运算并输出小写 hex 摘要；`-md5 FILE MD5` 再与期望值比对——相等输出 `match: <摘要>`，不等输出 `mismatch: computed <摘要> != expected <期望>`；`-sha` 同理（SHA-1）。期望值 trim + 小写后校验，长度或字符不合法报 `expected md5/sha1 must be N hex characters`。0 参数 / 超过 2 参数 / 文件不存在各有明确报错；`-md5` 与 `-sha` 同现报 `cannot combine -md5 and -sha`
- **自包含哈希实现（零新依赖）**：新增 `src/hash.rs`——`Md5` / `Sha1` 流式结构体（`update()` 任意分块安全、`finalize()` 填充）、`hash_file`（64 KiB 分块流式读大文件，内存有界）。clap 不支持多字符短参数，故 `-md5`/`-sha` 在 `FileArgs.args`（`allow_hyphen_values = true`）内由 `split_hash_args` 自行识别（`-md5`/`--md5` 均接受，`-md5x` 之类不误认），标记可出现在任意位置
- 文档同步：README.md / README.en.md（选项与示例）、src/file.rs 头部 Usage/Options；版本 0.5.3 -> 0.5.4；`cargo test` 263 -> 272 项全绿（新增 hash.rs 7 项 + file.rs 2 项）

### 修复
- **MD5 全量测试向量错误根因：K 表第 28 项拼写错位**（`0xa3e905fa` -> 正确值 `0xa9e3e905` = floor(sin(29)×2³²)）。MD5 每个消息都经过第 28 轮，故空串/abc/fox/百万 a 全部错算；修正后全部命中 RFC 1321 向量。SHA-1 实现本身正确（FIPS 180-1），此前 "fox 失败" 实为测试期望值笔误——该字符串真实 SHA-1 为 `2fd4e1c67a2d28fced849ee1bb76e7391b93eb12`（Windows CNG certutil / OpenSSL hashlib / .NET Get-FileHash 三方交叉确认），已按实际值修正断言

### 冒烟验证（真实二进制，Windows）
- `sys file -md5 <文件>` 输出与 `certutil -hashfile <文件> MD5` 一致
- `sys file -sha <文件>` 输出与 `Get-FileHash <文件> -Algorithm SHA1` 一致
- `sys file -md5 <文件> <正确摘要>` -> `match: ...`；`sys file -md5 <文件> <错误摘要>` -> `mismatch: ...`；非法期望（长度/非 hex）-> `expected md5 must be 32 hex characters`
- `sys file -md5 -sha <文件>` -> `cannot combine -md5 and -sha`；`sys file -md5`（无参）-> 用法报错

---

## v0.5.3（2026-10-15）

### 新增
- **`sys file -l`（统计行数）**：`-l FILE` 统计文件的总行数（存在即可，不要求带扩展名）；无参数时统计 stdin 的总行数（`type a.txt | sys file -l`）；带 PATTERN 时统计**匹配行数**——stdin 输出 `N`，目录树输出每个文件的 `路径: N`（无匹配的文件不显示，`rg -c` 风格）。引号包裹的参数（`"me.txt"`）仍强制按字符串搜索；3 参数替换模式与 `-l` 组合明确报错。行数按内容行计（末尾换行不产生幻影空行，`\r\n` 同样按行）
- **`sys file -n`（输出行号）**：stdin 搜索时在每行输出前加 `行号:内容`（上下文行也带行号）；目录搜索输出本身已是 `路径:行号:内容`，无需额外参数
- 文档同步：README.md / README.en.md（选项与示例）、src/file.rs 头部 Usage/Options；版本 0.5.2 -> 0.5.3；`cargo test` 259 -> 263 项全绿（新增 `build_output_line_numbers_stdin`、`count_lines_text_edges`、`count_matching_lines_flags`、`count_file_lines_counts_existing_file`）

### 冒烟验证（真实二进制，Windows）
- `sys file -l README.md` -> 总行数；`Get-Content x | sys file -l` -> stdin 总行数
- `type a.txt | sys file -n hello` -> `行号:内容`；`sys file -l hello <树>` -> 每文件 `路径: N`
- `sys file -l a b c`（3 参数）-> 明确报错 `-l (count lines) does not apply to replace mode`

---

## v0.5.2（2026-10-14）

### 新增
- **`sys path unlock TARGET`（Windows）**：检查文件/目录是否被其他进程占用/锁定，列出占用进程（PID / 进程名 / 可执行文件完整路径），并强制关闭占用进程释放句柄解锁。基于 Windows Restart Manager（Rstrtmgr.dll）：`RmRegisterResources` 注册目标 → `RmGetList` 枚举占用方 → `RmShutdown(RmForceShutdown)` 关闭 → 重查验证（每次开**全新会话**查真实状态，最多 20 次 × 300ms ≈ 6s，进程终止为异步、成功即提前退出）。未占用输出 `not locked: <路径>`；解锁成功输出 `unlocked: <路径>`；仍有残留打印剩余占用方并以错误退出。非 Windows 报 `path unlock is Windows-only`（Linux 文件锁为 advisory 无需解锁）
- windows-sys 新增启用 feature：`Win32_System_RestartManager`、`Win32_Storage_FileSystem`、`Win32_Security`
- README.md / README.en.md 同步（命令列表、特性表、PATH 示例）；版本 0.5.1 -> 0.5.2；`cargo test` 257 -> 259 项全绿（新增 `lock::tests::no_handle_is_not_locked`、`lock::tests::open_handle_is_reported`，仅覆盖检测路径；解锁动作由真实冒烟验证，不做会自杀的单测）

### 冒烟验证（真实二进制，Windows）
- `sys path unlock <未被占用的文件>` → `not locked: ... (no process has it open)`
- 记事本打开文件后 `sys path unlock <该文件>` → 列出 `notepad.exe`（PID/路径）→ 关闭后输出 `unlocked: ...`，随后再查报 `not locked`（句柄确已释放）
- `sys path unlock <不存在的路径>` → 报 `does not exist or is not accessible`

---

## v0.5.1（2026-10-13）

### 变更
- **`sys path add` 默认仅当前会话，`--save` 才持久化**：`sys path add DIR` 不再写入注册表/持久化存储，只更新当前进程并打印一条可粘贴片段（`export` / `$env:`）供当前 shell 应用；加 `--save` 才持久化（默认 user 作用域，`--scope machine` 需管理员/root）。原 `--temporary` 标志移除（与默认行为重复）
- **修复 `--temporary` "看起来没生效" 的体验问题**：原实现在临时分支把片段打印两遍（`apply_to_current_env` + hint），且默认行为是持久化、`--temporary` 只打印片段——子进程无法修改父 shell 环境，用户感知为"没生效"。新默认把"仅当前会话"提到首位，输出收敛为一条片段；持久化分支同样只打印一次片段
- README.md / README.en.md 同步；版本 0.5.0 -> 0.5.1；`cargo test` 257 项全绿（无新增测试，纯行为翻转 + 输出收敛，冒烟验证见下）

### 冒烟验证（真实二进制，Windows）
- `sys path add D:\x` → 输出 `Temporary: ... added to PATH for this session (not saved).` + 单条 `$env:PATH` 片段；新进程 `sys path has D:\x` 返回 `no`（未持久化）
- `sys path add --save D:\x` → 持久化 user 作用域，新进程 `has` 返回 `yes`；随后 `path remove` 清理，`has` 返回 `no`（注册表净零）
- `sys path add --temporary` → clap 报 `unexpected argument '--temporary'`

---

## v0.5.0（2026-10-12）

### 变更
- **`sys ai config` 缺省导出模型与 `sys ai chat` 一致**：不加 `-m` / 位置参数时，按配置顶层 `model:` 选择器解析导出（未配置则取第一个 Provider 的第一个模型），不再导出全部模型；显式 `-m` / 位置参数行为不变
- 版本 0.4.72 -> 0.5.0；`cargo test` 256 -> 257 项全绿

---

## v0.4.72（2026-10-12）

### 新增
- **`sys ai config` 新增 codebuddy / workbuddy / trae 导出**：codebuddy / workbuddy 输出 models.json（`~/.codebuddy/models.json` / `~/.workbuddy/models.json`，同 schema：`models` + `availableModels`，每条 `id`/`name`/`vendor`/`url`/`apiKey`，可选 `maxInputTokens`/`maxOutputTokens`/`supportsImages`，同名模型去重）；trae 输出 trae_cli.yaml（Windows `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`、Linux `$XDG_CONFIG_HOME` 或 `~/.config/trae_cli/trae_cli.yaml`，OpenAI 兼容 → `open_ai` 块、anthropic → `claude` 块）
- **`-m/--models` 多模型选择**：与 `sys ai chat -m` 完全相同的格式（`provider:model` / `provider:*` / 裸模型名，半角/全角逗号多选，重叠去重），复用 `chat::resolve_targets`；原位置参数 `SELECT` 保留兼容（与 `-m` 互斥）
- **`--update` 就地合并更新**：按格式到该软件默认配置目录找文件（`-f` 覆盖路径）；文件不存在提示 `config file not found ... nothing to update`；存在则先备份 `.bak-*`（不覆盖旧备份），合并写入缺失模型（保留已有条目与其它顶层键），成功提示 `added N model(s): ...`；仅 codebuddy / workbuddy / trae 支持，与 `--show` 互斥
- 版本 0.4.71 -> 0.4.72；`cargo test` 247 -> 256 项全绿

### 修复
- **合并解析前剥离 UTF-8 BOM**（Windows 编辑器保存的 JSON/YAML 常见，否则严格解析报 `expected value at line 1 column 1`）

---

## v0.4.71（2026-10-12）

### 新增
- **`sys ai model --src openrouter`**：新增 `--src` 参数（默认 `models`，不提供时行为与旧版完全一致）；`--src openrouter` 时 HTTP GET 访问 `https://openrouter.ai/api/v1/models`，列表显示模型的 `id` / `name` / `context_length` / 价格（取 `pricing.prompt`，每 1M token）/ 创建时间（`created` unix 时间戳转为本地时间 `YYYY-MM-DD HH:MM:SS`）；**支持模型名称过滤**：`sys ai model deepseek --src openrouter` / `-s 关键词` 按名称筛选（先精确后子串，逗号分隔多词任一命中即保留），不提供时全列表；支持 `--limit N` / `--json` / `-o json|csv|yaml|md`；价格显示保留足够有效位（8 位小数并去尾零）
- **`sys ai price` 独立命令（完全接管 `info price`）**：`sys ai price P1,P2,...` 查 models.dev 价格（`--refresh` / 文本表，行为同原 `info price`）；新增 `--src` 参数（默认 `models`）；`--src openrouter` 时访问 openrouter.ai `/api/v1/models`，将每个模型 `pricing` 的 `prompt` 与 `completion` 作为价格展示（每 1M token，USD），**可选位置参数作模型名称过滤**（`sys ai price deepseek --src openrouter`），不提供时全列表；支持 `--limit` / `--json` / `-o json|csv|yaml|md`
- **删除 `sys ai info price`**：功能与 `sys ai price` 重复，`info` 不再接受 `price` 字段（报错提示迁移到 `sys ai price`）
- README.md / README.en.md 同步用法；版本 0.4.70 -> 0.4.71；`cargo test` 247 项全绿

### 修复
- **OpenRouter 价格单位修正（per-token → per 1M）**：openrouter.ai `pricing` 字段为每 token USD，此前文本/MD 视图直接展示该值但表头标注 "per 1M tokens"，数值差 1e6；现在所有输出格式（文本 / md / csv / json / yaml）统一换算为每 1M token 价格
- **`ai model --src openrouter` 的 JSON / YAML 输出忽略过滤与 `--limit`**：原分支遍历全量 `items`（458 个）而非过滤后的行，`sys ai model deepseek --src openrouter --json` 会输出全部模型；改为与 csv/md/文本一致基于过滤 + 截断后的行，`OrModelRow` 新增 `created_ts` 保留原始时间戳
- **openrouter 目录加入 24h 本地缓存与 30s 请求超时**（原先每次调用实时 GET 全量目录且无超时，`--refresh` 强制重抓）
- **`--src openrouter` 与其他过滤参数冲突时明确报错**：`--list` / `--provider` / `--price` / `--open` / `--model-type` / `--date` 仅适用于 models.dev 源，此前被静默忽略
- **`sys ai price` 多位置参数按逗号拼接**：`sys ai price a b --src openrouter` 与 `a,b` 同义（原先拼成一个带空格的查询词）
- **文档同步**：命令注释 `sys ai models` → `sys ai model`；`info` 模块注释与 README 总览中的过时 `info price` 指引改为指向 `sys ai price`
- 新增 3 个单测（价格单位换算、过滤精确优先/子串、JSON 视图过滤与 limit）；`cargo test` 241 -> 247 项全绿

---

## v0.4.70（2026-10-11）

### 新增
- **`sys ai cn-model` 模型信息包含模型规模（参数量）**：单模型精确命中抓详情页时，新增 `parameters` 字段（来源：DataLearner 详情页「总参数 / 激活参数」行，如 `2350亿 / 220亿`），与既有 `context` / `modality` 一样仅在详情抓取时填充；详情视图新增 `parameters` 行、`-o md` 表格新增 `parameters` 列、CSV 扩展为 11 列（`...context,modality,parameters,published,url`）、JSON / YAML 自动携带；站点「暂无数据」占位视为缺失（部分已知值如 `2350亿 / 暂无数据` 保留）；`DlModel.parameters` 带 `#[serde(default)]` 兼容旧缓存
- **修复 `ai cn-model` 详情页字段解析 bug**：`dl_detail_value` 原取标签首次出现位置，真实页面标签常先出现在 meta description，导致 `context` 抓到页面导航文本；改为逐次扫描、仅接受标签后 ≤200 字节内紧跟值行的命中，`上下文长度` / `输入/输出模态` / `总参数 / 激活参数` 共用此逻辑
- **`ai model`（models.dev）结论记录**：经核实 models.dev API 与网页均无结构化模型规模字段，按「如果网站提供」条件维持现状
- 新增 1 个单测并扩展 2 个（`dl_detail_value_skips_meta_description_hits`、`总参数 / 激活参数` 与占位值解析、CSV 11 列断言）；`cargo test` 244 项全绿
- 版本 0.4.69 -> 0.4.70

---

## v0.4.69（2026-10-11）

### 新增
- **`sys ai chat --query-model PROVIDER`：查询 provider 的模型列表**（调用其 `{api_base}/v1/models` 接口）：
  - PROVIDER 为配置 `clients` 下的 provider 名称（如 `modelscope`）；不存在时列出可用 provider
  - URL 拼法：`api_base` 已以 `/v1` 结尾则拼 `/models`，否则拼 `/v1/models`
  - 认证与 chat 一致（openai 用 Bearer；anthropic 用 `x-api-key` + `anthropic-version`）；连接 10s / 总 30s 超时
  - 响应解析：优先 `data[]`、回退 `models[]`，条目取 `id`（含可选 `owned_by`）；`error` 对象报错；解析前剥离 UTF-8 BOM；支持 `next_page` 分页（上限 20 页）；重复 id 去重
  - 输出：`provider (kind) — url  (N model(s) via API)` 后跟每行 `- id`
- 新增 7 个单测（models_url 拼法、data[]/models[] 解析、BOM 剥离与错误形状、本地 HTTP 抓取、403 报错、next_page 分页、未知 provider 报错）；`cargo test` 243 项全绿
- 版本 0.4.68 -> 0.4.69

---

## v0.4.68（2026-10-11）

### 新增
- **`--test` 三项行为调整**：
  - **`--remove` 删除前自动备份原始配置文件**：`apply_test_removals` 在真正删除前先把配置复制为同目录下的 `config.yaml.bak-<时间戳>`（如 `config.yaml.bak-20261011-203246`），并输出 `sys: backed up original config to <路径>`；没有失败（无需删除）时不产生备份
  - **每测试完一个模型立即打印该条结果**：不再等全部模型测完按输入顺序统一输出，改为完成一个打印一个（完成顺序）；worker 仍按输入序拉取下一条目标
  - **默认并行数改为 provider 数量**（上限为 CPU 核心数）：`--parallel N` 仍优先（上限为模型数）；头部输出 `testing N model(s) across P provider(s) in parallel (W worker(s))`
- 新增 3 个单测（删除前备份内容与原文件一致、无删除时不备份、默认并行数=provider 数且受 CPU 上限/显式参数覆盖）；`cargo test` 236 项全绿
- 版本 0.4.67 -> 0.4.68

---

## v0.4.67（2026-10-11）

### 新增
- **`sys ai chat --test --remove`：测试失败的模型直接从配置文件删除**（替代置 weight=0）：
  - `--remove` 必须与 `--test` 同用（clap `requires`）；失败模型（HTTP 错误如 429、连接错误、超时、无内容反馈）的条目从 `clients` 中删除，而非写 `weight: 0`
  - 同一 provider 下同名模型条目按出现次序精确定位删除（`remove_model_entry_from_config` 的 occurrence 参数）
  - **删除后无剩余模型的 provider 整体移除**（`remove_provider_entry_from_config`，含 api_key 等整块配置）
  - 顶层 `model:` 选择器清理（`clean_top_level_model_selector`）：仍能解析到剩余模型的 selector 保留，悬空引用（已删模型 / 已删 provider）剔除，清空则删除 `model:` 行
  - 输出 `[FAIL] ...  (removed)`，汇总 `... failed (removed from config)`；写回失败仅警告、不中断；删除串行执行避免并发改配置
- 新增 6 个单测（模型条目删除保留兄弟模型、mangled `- weight:` 同删、同名 occurrence 定位、provider 整块删除、顶层 selector 清理、`apply_test_removals` 编排）；`cargo test` 233 项全绿
- 版本 0.4.66 -> 0.4.67

---

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
