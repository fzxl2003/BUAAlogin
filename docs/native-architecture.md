# 原生实现

Cargo workspace 的 `buaa-core` 提供认证、HTTP、接口发现、调度和 C ABI；`buaalogin` 提供非交互 CLI；`buaalogin-windows` 是直接调用 Win32 的窗口／托盘程序。macOS 的 Swift/AppKit 入口静态链接 core。桌面构建通过 `browser-fallback` feature 按需启动本机 Chromium 系浏览器；CLI/Docker 关闭该能力。

## 会话与网络

每个接口按名称、IPv4、系统接口索引唯一标识，拥有独立 libcurl easy handle、内存 Cookie、页面 IP/ac_id、连接缓存、检查截止时间和失败次数。认证状态由 `rad_user_info` 获取，外网状态由独立 easy handle 并发探测两个 HTTPS 站点；任一 2xx 通过，重定向不通过。外网通过时不因认证查询失败而注销。断线后重新读取页面和 challenge；网关页面返回的 IP 是校园网可见地址，与 challenge 和状态返回的 IP 校验一致；它可以与本地接口地址不同（路由器 NAT），提交成功后再次检查认证记录及外网。

认证请求固定为 `https://gw.buaa.edu.cn:443`，通过 resolve 将域名指向配置中的网关 IPv4。外网 HEAD 仅访问代码中固定的百度、腾讯 HTTPS 站点，不携带门户 Cookie。源地址由 libcurl interface 绑定；socket 创建时额外设置 macOS IP_BOUND_IF 或 Windows/Linux IP_UNICAST_IF。接口绑定失败直接报错，不退回系统默认路由。禁用代理和 TCP keepalive，不关闭证书校验。限制响应体为 1 MiB，认证请求不接受其他域名、端口或 HTTP 跳转。

三轮外网失败之间至少间隔 10 秒；共享校园网出口的多个接口不会使轮数加速。恢复按协议注销、等待 2 秒、清 Cookie 后强制登录、外网验证、网页操作、外网验证执行。协议重登绕过 online 快速路径。`RecoveryBook` 位于接口 Slot 之外，以网关和校园网 IP 标识出口，开始恢复前预留 600 秒冷却；接口刷新不清除冷却。账号限制等结构化 `NeedsUser` 错误停止升级，网关原始描述不进入日志。

网页恢复复用本机 Chrome/Edge/Chromium，以临时 profile 和仅本机随机调试端口启动。Rust 通过 CDP 读取主页面控件、发送 Input 事件，确认字段焦点和点击目标；支持确认弹窗，歧义、验证码或跨源页面明确失败。临时 CONNECT 代理只允许网关域名 443 端口，上游复用相同的 socket 接口绑定和配置网关 IP，不终止 TLS。浏览器禁用代理绕过、QUIC、非代理 WebRTC 和背景网络功能。临时浏览器目录、代理、调试连接、Unix 进程组/Windows Job Object 均随作用域清理，取消 generation 与 90 秒预算覆盖网页恢复及后续验证。

macOS 和 Linux 通过 getifaddrs 获取接口快照（Linux libc 通过 rtnetlink 读取）；Windows 使用 GetAdaptersAddresses。仅启动、手动检测或网络通知后刷新快照。macOS 使用 SCDynamicStore，Windows 注册接口和单播地址变化，Linux 阻塞等待 rtnetlink 的 link/address/route 通知。接口通知仅重建受影响会话；无法确定接口的全局路由变化重建全部候选会话。桌面默认优先上次成功接口；Docker 每个接口独立调度。

Srun xencode 遵循 JavaScript UTF-16 code units，HMAC/SHA1 使用 UTF-8。BMP 与补充平面字符（如 emoji）均按 JavaScript charCodeAt 语义处理。固定向量位于 core 测试目录，不包含真实凭据。

## 调度与生命周期

监控线程串行执行请求，同一接口不会重入。网络事件在接收时增加取消 generation，再合并 2 秒；请求的 libcurl progress callback 检测 generation 并中止传输，空闲等待由命令通道立即唤醒。取消依赖 libcurl 的 progress 调用，在活跃请求中通常有约一秒延迟，不等待完整 15 秒超时。

macOS 监控线程阻塞等待命令，由 UI 收到 schedule 事件后设置一个单次 GCD timer，普通在线检查允许 min(5, interval / 10) 秒 leeway，最小周期 10 秒时仅允许 1 秒合并窗口，重试和网络去抖为 0。Windows/Linux 使用单次通道截止时间等待，不创建周期轮询。桌面 suspend 取消请求并丢弃会话／截止时间，resume 合并触发一次刷新。无候选接口时不安排重试，只等待网络通知或停止命令。

每个接口按 30、60、120、240、300 秒退避，成功清零，恢复配置周期。重试事件携带下一次重试的时间戳，桌面显示本地时间；每次失败更新一次，不刷新倒计时。每个接口分别抑制重复在线状态日志；桌面历史最多 120 行。Rust 保存的密码使用 Zeroizing，停止释放监控配置。GUI／系统库的内部字符串缓冲区无法承诺完全清零。

## C ABI v1

`platform/macos/BUAACore.h` 是接口定义：`buaa_abi_version`、`buaa_create`、`buaa_start`、`buaa_scan`、`buaa_stop`、网络变化／睡眠／恢复／tick 通知及 `buaa_free`。

- create 接受仅在调用期间借用的 UTF-8 NUL 结尾 JSON、回调和上下文；失败返回 null。配置字段对应 Rust Config：username、password、interface（可省略）、gateway_ip、interval、try_all、external_timer、portal_auth_enabled、connectivity_probe_enabled、browser_fallback、browser_path。两个新增任务开关缺失时默认开启；browser_path 缺失时自动发现浏览器。
- `buaa_test(client, kind)` 中 kind=0/1/2 分别执行认证、外网和网页真实测试；监控运行时排入串行命令队列，空闲时启动一次性任务。
- start/scan 返回 0 表示启动成功，负数表示无效句柄、任务仍运行、配置或启动失败、内部 panic。一个句柄同时只能执行一个任务。
- `buaa_recover(client, mode)` 增量扩展 ABI v1：mode=0 请求完整协议恢复链，mode=1 直接请求网页兜底；返回 0 表示已排队，-1 为无效句柄，-2 为任务未运行，-3 为无效 mode，-4 为内部异常。恢复在监控线程串行执行、合并重复待处理请求，且保留出口冷却。
- 句柄由创建它的主线程串行调用；不得并发 start/free 或重复 free。NULL 句柄可用于通知／free 空操作。callback 在工作线程调用，JSON 指针只在回调期间有效，调用者应先复制再切换主线程。
- 事件包含 state、message、可选 records、可选 delay、可选 retry_at、可选 dashboard 和 leeway。schedule 中 delay 缺失意味着取消定时器；interfaces 返回含 name/address/index/binding/label/campus/campus_ip/message 的列表。
- callback 不可 unwind，不可在工作线程内部调用 free。所有 ABI 入口捕获 Rust panic；回调和上下文需存活到 free 返回。free 取消并 join 工作线程，返回后不会再产生回调。
- Swift 使用 epoch 丢弃已入队的旧任务事件；退出时释放句柄、定时器、网络与睡眠通知注册。

## 原生界面

macOS 使用 AppKit 系统字体、动态配色、SF Symbols、分段控件和原生 sheet；Windows 保留 Common Controls v6 标签页及标准输入控件。默认内容区域为 680 × 700，只有“连接”和“账号设置”两页。连接页提供单一开始／停止按钮、五项连接信息、带高对比状态圆点的三行任务状态和最近五条活动；完整记录按日写入系统用户日志目录并保留 30 天。

Windows 输入框仍由原生 Edit 负责输入法、密码遮蔽和辅助技术；外框及焦点强调色由设置面板绘制。Tab 和方向键沿用原生控件交互；Enter 优先激活当前获焦的按钮，编辑字段中保留默认启动行为。按钮动作只接收 BN_CLICKED，焦点通知只用于滚入视口。系统高对比度启用时使用系统颜色，主题通知触发重绘。不加载 Shell 图标资源，程序与托盘图标在启动时用 GDI 生成一次；自建字体、图标及临时绘图对象均配对释放。

设置表单使用独立原生滚动区域，按账号和连接分组，高级选项默认折叠。网关校验出错时展开高级选项、滚动并聚焦该字段。账号、连接字段在任务运行时锁定，Wi-Fi 开关保持原有即时生效行为。启动成功返回仪表盘；tab 切换、日志和高级选项展开不触发网络操作。

`dashboard` 是只读事件，其快照除原有连接字段外包含 `tasks.portal_auth`、`tasks.connectivity` 和 `tasks.browser_fallback`。每项任务提供 enabled、phase、message、last_run_at、next_run_at 与 schedule_reason；未知时间为 null。C ABI v1 通过新增的 `buaa_test(client, kind)` 兼容扩展支持认证、外网和网页三类真实测试。

界面不使用网页视图、重绘定时器或循环动画；数据事件到来才更新。仪表盘隐藏时仅维护活动历史，返回后再渲染日志。

## 登录自启

开关默认关闭。macOS 13+ 使用 SMAppService.mainApp，状态由系统读取，requiresApproval 时展示登录项设置入口；macOS 12 原子写入用户 Library/LaunchAgents/cn.buaa.autologin.native.v1.login.plist，RunAtLoad + Aqua 会话 + --autostart，不使用 KeepAlive、定时启动或 launchctl。系统升级到 macOS 13 时将本程序的旧登录项迁移到 SMAppService。Windows 通过 Registry API 修改 HKCU Software\Microsoft\Windows\CurrentVersion\Run 中独立的 BUAALoginNativeV1 值，命令为带引号的完整 EXE 路径 + --autostart；命令中不含账号或密码。

自启来源由 macOS 登录 Apple Event／--autostart，Windows --autostart 判断。读取原生凭据、间隔、网关和指定接口；只有已保存完整凭据及有效配置才调用监控入口，并保持窗口隐藏。否则打开设置并提示，不先启动 Inspect。指定接口恢复时若暂不可用，保留占位选项，禁止静默切换为自动选择。正常手动启动保持原有界面与检测流程。macOS 检查已运行 bundle 实例并处理 reopen；Windows 使用当前会话 Local\BUAALoginNativeV1 mutex，阻止重复进程监控。

## 桌面 Wi-Fi 自动连接

macOS CoreWLAN 枚举所有 Wi-Fi 网卡（不依赖 IPv4），按接口名称保存启用选项；Windows Native WLAN 按接口 GUID 保存。默认关闭，只在自动重连运行且非睡眠时启用。启用意味着发现目标 SSID 时允许切换当前无线网络；停止后不主动断开已连接的网络。

macOS 使用 CWWiFiClient 的扫描缓存、SSID、链路和电源通知，在 utility 队列扫描及关联；CoreLocation 权限只在用户启用并运行功能时申请。Windows 使用 WlanScan / WlanGetAvailableNetworkList / WlanConnect 和 ACM 回调，独立阻塞事件线程执行无线 API。没有按 10 秒认证周期进行 RF 扫描；连接请求按接口节流 30 秒。每张网卡独立保留配置，用户关闭功能后不再提交新连接请求；已经提交给系统的无线关联操作可能仍会完成。

开放网络直接关联；Windows 也可复用已有系统无线 profile。macOS 遇到加密的同名网络提示用户在系统中处理，不将网页认证密码当作 PSK 或 802.1X 密码。系统权限或服务不足时输出明确状态，其他接口的认证监控继续运行。

## 构建与回退

构建脚本只打包原生程序，从独立 staging 目录生成 macOS app，避免混入其他构建产物。Windows 内嵌 asInvoker／系统 DPI／原生控件 manifest。凭据标识使用新的 native/v1 命名空间，与旧钥匙串及 DPAPI 文件隔离。

默认 CI 发布预览版，`promote_native` 只用于完成验收后的正式替换。发布前检查两个桌面包及两种 Linux 架构；仓库已移除所有 Python 旧实现及旧打包目录；回退使用之前发布的发行版或 Git 提交。
