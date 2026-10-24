"use strict";

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const app = { csrf: "", state: null, events: null, modalSubmit: null, modalKind: null, interfaceSignature: "" };

document.addEventListener("DOMContentLoaded", boot);

async function boot() {
  bindStaticEvents();
  applySavedTheme();
  try {
    const bootstrap = await request("/api/bootstrap", { auth: false });
    if (bootstrap.needs_setup) return showAuth(true);
    const session = await request("/api/session");
    enterApp(session.csrf);
  } catch (error) {
    if (error.status === 401) showAuth(false);
    else showAuth(false, error.message);
  }
}

function bindStaticEvents() {
  $("#auth-form").addEventListener("submit", submitAuth);
  $$(".tab").forEach(button => button.addEventListener("click", () => showPage(button.dataset.page)));
  $("#theme-button").addEventListener("click", toggleTheme);
  $("#start-button").addEventListener("click", toggleMonitoring);
  $("#account-button").addEventListener("click", openAccountModal);
  $("#interfaces-button").addEventListener("click", openInterfacesModal);
  $("#interval-button").addEventListener("click", openIntervalModal);
  $("#traffic-button").addEventListener("click", openTrafficModal);
  $("#password-button").addEventListener("click", openPasswordModal);
  $("#logout-button").addEventListener("click", logout);
  $("#portal-enabled").addEventListener("change", saveTaskSettings);
  $("#connectivity-enabled").addEventListener("change", saveTaskSettings);
  $$(".test-button").forEach(button => button.addEventListener("click", () => runTest(button)));
  $("#modal-form").addEventListener("submit", event => { event.preventDefault(); app.modalSubmit?.(); });
  $("#modal-close").addEventListener("click", closeModal);
  $("#modal-cancel").addEventListener("click", closeModal);
  $("#modal-backdrop").addEventListener("click", event => { if (event.target === event.currentTarget) closeModal(); });
  document.addEventListener("keydown", event => { if (event.key === "Escape") closeModal(); });
}

function showAuth(setup, error = "") {
  app.events?.close();
  $("#app-view").hidden = true;
  $("#auth-view").hidden = false;
  $("#auth-title").textContent = setup ? "首次设置" : "登录网页管理端";
  $("#auth-hint").textContent = setup
    ? "从容器日志中找到一次性设置码，并创建至少 8 个字符的管理密码。"
    : "输入网页管理密码以查看状态并控制自动连接。";
  $("#setup-code-field").hidden = !setup;
  $("#setup-code").required = setup;
  $("#auth-password").autocomplete = setup ? "new-password" : "current-password";
  $("#auth-password-label").textContent = setup ? "创建管理密码" : "管理密码";
  $("#auth-submit").textContent = setup ? "完成设置" : "登录";
  $("#auth-form").dataset.setup = setup ? "true" : "false";
  $("#auth-error").textContent = error;
  (setup ? $("#setup-code") : $("#auth-password")).focus();
}

async function submitAuth(event) {
  event.preventDefault();
  const setup = event.currentTarget.dataset.setup === "true";
  const button = $("#auth-submit");
  button.disabled = true;
  $("#auth-error").textContent = "";
  try {
    const payload = setup
      ? { token: $("#setup-code").value.trim(), password: $("#auth-password").value }
      : { password: $("#auth-password").value };
    const result = await request(setup ? "/api/setup" : "/api/login", { method: "POST", body: payload, auth: false });
    $("#auth-form").reset();
    enterApp(result.csrf);
  } catch (error) {
    $("#auth-error").textContent = error.message;
  } finally { button.disabled = false; }
}

async function enterApp(csrf) {
  app.csrf = csrf;
  $("#auth-view").hidden = true;
  $("#app-view").hidden = false;
  try {
    renderState(await request("/api/state"));
    connectEvents();
  } catch (error) {
    if (error.status === 401) showAuth(false, "登录已过期，请重新登录。");
    else toast(error.message);
  }
}

function connectEvents() {
  app.events?.close();
  const stream = new EventSource("/api/events");
  stream.addEventListener("state", event => {
    try { renderState(JSON.parse(event.data)); } catch (_) { refreshState(); }
  });
  stream.addEventListener("refresh", refreshState);
  stream.onerror = () => { /* EventSource reconnects automatically and the server sends a fresh snapshot on gaps. */ };
  app.events = stream;
}

async function refreshState() {
  try { renderState(await request("/api/state")); } catch (error) { if (error.status === 401) showAuth(false); }
}

function renderState(state) {
  app.state = state;
  const dashboard = state.dashboard || {};
  const locked = state.monitoring || state.busy;
  $("#state-title").textContent = phaseTitle(dashboard.phase || state.phase);
  $("#state-message").textContent = state.message || "—";
  const icon = $("#state-icon");
  icon.className = `state-icon ${phaseClass(dashboard.phase || state.phase)}`;
  icon.textContent = phaseGlyph(dashboard.phase || state.phase);
  const start = $("#start-button");
  start.textContent = state.monitoring ? "停止自动连接" : "开始自动连接";
  start.disabled = state.busy && !state.monitoring;
  $("#metric-account").textContent = state.account.username || "未设置";
  $("#metric-interface").textContent = dashboard.interface || selectedInterfaceText(state.settings.interfaces);
  $("#metric-interval").textContent = `${state.settings.interval} 秒`;
  $("#metric-last-check").textContent = formatTime(dashboard.last_check_at);
  $("#metric-local-ip").textContent = dashboard.local_ip || "—";
  $("#metric-campus-ip").textContent = dashboard.campus_ip || "—";
  $("#account-button").disabled = locked;
  $("#interfaces-button").disabled = locked;
  renderTraffic(dashboard.traffic_used_bytes, state.settings.traffic_total_gb);
  renderTasks(state, dashboard.tasks || {});
  renderActivities(state.activities || []);
  if (app.modalKind === "interfaces") maybeRefreshInterfaceModal();
}

function renderTraffic(used, totalText) {
  const totalGb = Number(totalText);
  const hasTotal = Number.isFinite(totalGb) && totalGb > 0;
  const hasUsed = Number.isFinite(used);
  const percent = hasUsed && hasTotal ? Math.min(100, used / (totalGb * 1e9) * 100) : 0;
  $("#traffic-bar").style.width = `${percent}%`;
  $("#traffic-percent").textContent = hasUsed && hasTotal ? `${percent.toFixed(percent < 10 ? 1 : 0)}%` : "—";
  $("#traffic-label").textContent = hasUsed
    ? hasTotal ? `已用 ${formatBytes(used)} / ${totalText} GB` : `已用 ${formatBytes(used)} · 请在设置页设置总流量`
    : "尚未获取流量";
  $("#traffic-summary").textContent = hasTotal ? `当前总流量：${totalText} GB` : "未设置总流量；设置后连接页会显示使用比例。";
}

function renderTasks(state, tasks) {
  const locked = state.monitoring || state.busy;
  const portal = $("#portal-enabled");
  const connectivity = $("#connectivity-enabled");
  portal.checked = state.settings.portal_auth_enabled;
  connectivity.checked = state.settings.connectivity_probe_enabled;
  portal.disabled = locked;
  connectivity.disabled = locked;
  renderTask("portal", tasks.portal_auth, state, state.dashboard?.authentication);
  renderTask("connectivity", tasks.connectivity, state, state.dashboard?.connectivity);
  $$(".test-button").forEach(button => {
    const active = state.testing === button.dataset.test;
    button.textContent = active ? "测试中…" : "测试";
    button.disabled = state.busy || Boolean(state.testing);
  });
}

function renderTask(name, task = {}, state, fallback) {
  const statusMap = { online: "已认证", offline: "未认证", available: "外网可用", unavailable: "外网不可用", unknown: "未知" };
  const status = task.message || statusMap[fallback] || (state.monitoring ? "等待检查" : "尚未运行");
  $("#" + name + "-status").textContent = status;
  const reason = task.schedule_reason;
  $("#" + name + "-next").textContent = task.next_run_at ? `下次 ${formatTime(task.next_run_at)}` : ({ disabled: "已关闭", on_start: "启动后立即" }[reason] || "—");
}

function renderActivities(activities) {
  const list = $("#activity-list");
  list.replaceChildren();
  if (!activities.length) {
    const empty = document.createElement("li"); empty.className = "empty"; empty.textContent = "暂无活动记录"; list.append(empty); return;
  }
  [...activities].reverse().slice(0, 40).forEach(activity => {
    const item = document.createElement("li");
    const time = document.createElement("time"); time.dateTime = new Date(activity.at * 1000).toISOString(); time.textContent = formatTime(activity.at);
    const message = document.createElement("span"); message.textContent = activity.message;
    item.append(time, message); list.append(item);
  });
}

async function toggleMonitoring() {
  const button = $("#start-button");
  button.disabled = true;
  try {
    await write(`/api/actions/${app.state.monitoring ? "stop" : "start"}`, {});
    toast(app.state.monitoring ? "正在停止" : "自动连接已开始");
  } catch (error) { toast(error.message, true); }
  finally { button.disabled = false; }
}

async function runTest(button) {
  button.disabled = true;
  try { await write(`/api/tests/${button.dataset.test}`, {}); toast("测试已开始"); }
  catch (error) { toast(error.message, true); button.disabled = false; }
}

async function saveTaskSettings() {
  try {
    await saveSettings({
      portal_auth_enabled: $("#portal-enabled").checked,
      connectivity_probe_enabled: $("#connectivity-enabled").checked
    });
    toast("任务设置已保存");
  } catch (error) { toast(error.message, true); refreshState(); }
}

function openAccountModal() {
  const account = app.state.account;
  openModal("账号设置", "校园网认证所使用的账号。密码留空时保留当前密码。", `
    <label class="field"><span>账号</span><input id="form-username" value="${escapeHtml(account.username)}" autocomplete="username" required></label>
    <label class="field"><span>密码</span><input id="form-password" type="password" autocomplete="new-password" placeholder="${account.has_password ? "留空以保留当前密码" : "请输入校园网密码"}"></label>
    <label class="switch-label"><input id="form-remember" type="checkbox" ${account.remembered ? "checked" : ""}><i></i><span>加密保存账号密码</span></label>
  `, async () => {
    await write("/api/account", { username: $("#form-username").value, password: $("#form-password").value || null, remember: $("#form-remember").checked }, "PUT");
    closeModal(); toast("账号已保存");
  });
}

function openIntervalModal() {
  openModal("检查周期", "运行期间也可以修改，新周期会应用于后续调度。", `
    <label class="field"><span>周期（秒）</span><input id="form-interval" type="number" min="10" max="86400" step="1" value="${app.state.settings.interval}" required></label>
  `, async () => {
    await saveSettings({ interval: Number($("#form-interval").value) });
    closeModal(); toast("检查周期已更新");
  });
}

function openTrafficModal() {
  openModal("设置总流量", "用于计算连接页的流量进度；留空可关闭比例显示。", `
    <label class="field"><span>总流量（GB）</span><input id="form-traffic" type="number" min="0.001" step="any" value="${escapeHtml(app.state.settings.traffic_total_gb)}" placeholder="例如 100"></label>
  `, async () => {
    await saveSettings({ traffic_total_gb: $("#form-traffic").value.trim() });
    closeModal(); toast("总流量已保存");
  });
}

function openPasswordModal() {
  openModal("修改网页管理密码", "修改后其他会话会立即失效。新密码至少 8 个字符。", `
    <label class="field"><span>当前管理密码</span><input id="form-current-password" type="password" autocomplete="current-password" required></label>
    <label class="field"><span>新管理密码</span><input id="form-new-password" type="password" autocomplete="new-password" minlength="8" required></label>
    <label class="field"><span>确认新密码</span><input id="form-confirm-password" type="password" autocomplete="new-password" minlength="8" required></label>
  `, async () => {
    const next = $("#form-new-password").value;
    if (next !== $("#form-confirm-password").value) throw new Error("两次输入的新密码不一致");
    const result = await write("/api/admin-password", { current_password: $("#form-current-password").value, new_password: next }, "PUT");
    app.csrf = result.csrf; closeModal(); toast("管理密码已修改");
  });
}

function openInterfacesModal() {
  app.modalKind = "interfaces";
  app.interfaceSignature = "";
  openModal("接口设置", "空选择表示自动选择；显式多选时每个接口独立监控。容器仅识别 Wi‑Fi，不控制宿主机连接。", interfaceModalHtml(), saveInterfaces);
  app.modalKind = "interfaces";
  bindInspectButton();
  app.interfaceSignature = interfaceSignature();
}

function interfaceModalHtml() {
  const settings = app.state.settings;
  const selected = new Set(settings.interfaces);
  const records = [...app.state.interfaces];
  settings.interfaces.forEach(name => { if (!records.some(item => item.binding === name || item.name === name)) records.push({ name, binding: name, kind: "wired", message: "已保存，当前未检测到" }); });
  const options = records.length ? records.map(item => {
    const key = item.binding || item.name;
    const checked = selected.has(key);
    const kind = item.kind === "wifi" ? "Wi‑Fi" : "有线";
    const gateway = settings.interface_gateways[key] || "";
    return `<div class="interface-option" data-interface="${escapeHtml(key)}"><label class="interface-top"><input class="interface-check" type="checkbox" value="${escapeHtml(key)}" ${checked ? "checked" : ""}><strong>${escapeHtml(item.label || item.name)}</strong><span class="type-badge">${kind}</span></label><p class="interface-detail">${escapeHtml(item.message || "可用接口")}</p><label class="gateway-field" ${checked ? "" : "hidden"}><span>自定义网关（可选）</span><input class="interface-gateway" value="${escapeHtml(gateway)}" placeholder="留空使用默认网关"></label></div>`;
  }).join("") : `<p class="muted">尚未检测接口。点击“检测接口”获取容器可见的 IPv4 网络接口。</p>`;
  return `<div class="modal-toolbar"><label class="field"><span>默认网关</span><input id="form-gateway" value="${escapeHtml(settings.gateway_ip)}" required></label><button class="secondary" id="inspect-button" type="button">检测接口</button></div><p class="muted">不勾选任何项时自动选择</p><div class="check-list" id="interface-list">${options}</div>`;
}

function bindInspectButton() {
  $("#inspect-button")?.addEventListener("click", async event => {
    event.currentTarget.disabled = true;
    event.currentTarget.textContent = "检测中…";
    try { await write("/api/actions/inspect", {}); toast("正在检测接口"); }
    catch (error) { toast(error.message, true); event.currentTarget.disabled = false; event.currentTarget.textContent = "检测接口"; }
  });
  $$(".interface-check", $("#modal-body")).forEach(input => input.addEventListener("change", () => {
    input.closest(".interface-option").querySelector(".gateway-field").hidden = !input.checked;
  }));
}

function maybeRefreshInterfaceModal() {
  const signature = interfaceSignature();
  if (signature === app.interfaceSignature) {
    if (!app.state.busy) { const button = $("#inspect-button"); if (button) { button.disabled = false; button.textContent = "检测接口"; } }
    return;
  }
  const defaultGateway = $("#form-gateway")?.value;
  $("#modal-body").innerHTML = interfaceModalHtml();
  if (defaultGateway) $("#form-gateway").value = defaultGateway;
  bindInspectButton(); app.interfaceSignature = signature;
}

function interfaceSignature() { return JSON.stringify((app.state.interfaces || []).map(item => [item.binding, item.kind, item.message])); }

async function saveInterfaces() {
  const interfaces = $$(".interface-check:checked", $("#modal-body")).map(input => input.value);
  const gateways = {};
  $$(".interface-option", $("#modal-body")).forEach(option => {
    const input = $(".interface-check", option); const value = $(".interface-gateway", option)?.value.trim();
    if (input?.checked && value) gateways[input.value] = value;
  });
  await write("/api/interfaces", { gateway_ip: $("#form-gateway").value.trim(), interfaces, interface_gateways: gateways }, "PUT");
  closeModal(); toast("接口设置已保存");
}

async function saveSettings(overrides) {
  const settings = app.state.settings;
  return write("/api/settings", {
    interval: settings.interval,
    traffic_total_gb: settings.traffic_total_gb,
    portal_auth_enabled: settings.portal_auth_enabled,
    connectivity_probe_enabled: settings.connectivity_probe_enabled,
    ...overrides
  }, "PUT");
}

async function logout() {
  try { await write("/api/logout", {}); } catch (_) { /* clearing the local view is still safe */ }
  app.csrf = ""; showAuth(false);
}

function openModal(title, hint, body, submit) {
  app.modalKind = null;
  app.modalSubmit = async () => {
    const button = $("#modal-save"); button.disabled = true; $("#modal-error").textContent = "";
    try { await submit(); } catch (error) { $("#modal-error").textContent = error.message; }
    finally { button.disabled = false; }
  };
  $("#modal-title").textContent = title; $("#modal-hint").textContent = hint; $("#modal-body").innerHTML = body;
  $("#modal-error").textContent = ""; $("#modal-backdrop").hidden = false;
  setTimeout(() => $("input", $("#modal-body"))?.focus(), 0);
}

function closeModal() { $("#modal-backdrop").hidden = true; app.modalSubmit = null; app.modalKind = null; }

function showPage(name) {
  $$(".tab").forEach(tab => tab.classList.toggle("active", tab.dataset.page === name));
  $$(".page").forEach(page => page.classList.toggle("active", page.id === `page-${name}`));
}

async function write(path, body, method = "POST") { return request(path, { method, body, csrf: true }); }

async function request(path, options = {}) {
  const headers = { Accept: "application/json" };
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  if (options.csrf) headers["X-CSRF-Token"] = app.csrf;
  const response = await fetch(path, { method: options.method || "GET", headers, credentials: "same-origin", body: options.body === undefined ? undefined : JSON.stringify(options.body) });
  let data = {};
  try { data = await response.json(); } catch (_) { if (!response.ok) data.error = `请求失败（${response.status}）`; }
  if (!response.ok) { const error = new Error(data.error || `请求失败（${response.status}）`); error.status = response.status; throw error; }
  return data;
}

function phaseTitle(phase) {
  return ({ online: "网络正常", checking: "正在检查", suspect: "外网异常，正在复核", protocol_recovery: "协议恢复中", verifying: "正在验证外网", cooldown: "恢复冷却中", needs_user: "认证需要处理", retry: "等待自动重试", error: "连接需要处理", failed: "连接需要处理", waiting_network: "等待网络", stopped: "自动连接已停止", idle: "尚未开始" })[phase] || "自动连接";
}
function phaseClass(phase) { return phase === "online" ? "online" : (["checking", "protocol_recovery", "verifying"].includes(phase) ? "working" : (["suspect", "needs_user", "retry", "error", "failed"].includes(phase) ? "warning" : "")); }
function phaseGlyph(phase) { return phase === "online" ? "✓" : (["checking", "protocol_recovery", "verifying"].includes(phase) ? "↻" : (["suspect", "needs_user", "retry", "error", "failed"].includes(phase) ? "!" : "○")); }
function selectedInterfaceText(interfaces) { return interfaces.length ? interfaces.join("、") : "自动选择"; }
function formatTime(timestamp) { return timestamp ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date(timestamp * 1000)) : "—"; }
function formatBytes(bytes) { return new Intl.NumberFormat("zh-CN", { style: "unit", unit: bytes >= 1e9 ? "gigabyte" : bytes >= 1e6 ? "megabyte" : "kilobyte", unitDisplay: "short", maximumFractionDigits: 2 }).format(bytes / (bytes >= 1e9 ? 1e9 : bytes >= 1e6 ? 1e6 : 1e3)); }
function escapeHtml(value) { return String(value ?? "").replace(/[&<>'"]/g, character => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[character]); }
function toast(message, error = false) { const element = $("#toast"); element.textContent = message; element.style.background = error ? "#b93636" : "#1d232b"; element.hidden = false; clearTimeout(toast.timer); toast.timer = setTimeout(() => element.hidden = true, 2800); }
function applySavedTheme() { const theme = localStorage.getItem("buaa-theme"); if (theme) document.documentElement.dataset.theme = theme; }
function toggleTheme() { const current = document.documentElement.dataset.theme; const dark = current ? current !== "dark" : !matchMedia("(prefers-color-scheme: dark)").matches; document.documentElement.dataset.theme = dark ? "dark" : "light"; localStorage.setItem("buaa-theme", document.documentElement.dataset.theme); }
