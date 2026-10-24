(() => {
  const visible = element => {
    const box = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return box.width > 0 && box.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };
  const unique = elements => {
    const matches = [...new Set(elements)].filter(visible);
    if (matches.length > 1) throw new Error('ambiguous');
    return matches[0] || null;
  };
  const field = (selectors, predicate) => {
    for (const selector of selectors) {
      const element = unique([...document.querySelectorAll(selector)].filter(predicate));
      if (element) return element;
    }
    return null;
  };
  const button = (selectors, labels) => field(selectors, () => true) || unique(
    [...document.querySelectorAll('button,a,input[type=submit],input[type=button],[role=button]')]
      .filter(element => labels.includes((element.innerText || element.value || '').trim()))
  );
  const ready = element => !element.disabled && (typeof window.jQuery?._data !== 'function' || typeof element.onclick === 'function' || (window.jQuery._data(element, 'events')?.click || []).length > 0);
  try {
    window.__buaaControls = {};
    // The portal renders a type=button before its later scripts register the click handler.
    if (document.readyState === 'loading') return {state: 'loading'};
    if ([...document.querySelectorAll('input')].some(element => visible(element) && /captcha|verify.?code|验证码/i.test([element.id, element.name, element.placeholder].join(' ')))) return {state: 'needs_user'};
    const notices = [...document.querySelectorAll('[role=alert],.layui-layer-content,.error,.error-msg,.error-message,#error,#error_msg,.component.confirm.active .section,.dialog-detail.active .section')].filter(visible).map(element => element.innerText || '').join(' ');
    const code = notices.match(/\bE\d{3,6}\b|\b(?:password_error|account_disabled|balance_insufficient|captcha)\b/i)?.[0] || null;
    if (/密码错误|账号禁用|帐号禁用|余额不足|欠费|验证码|password.error|captcha|\bE2531\b/i.test(notices)) return {state: 'needs_user', code};
    if (/失败|错误|超时|无效|禁止|不存在|限制|error|fail|invalid|\bE\d{3,6}\b/i.test(notices)) return {state: 'portal_error', code};
    const dialog = unique([...document.querySelectorAll('.component.confirm.active,.dialog-detail.active')]);
    if (dialog) {
      const message = dialog.querySelector('.section')?.innerText || '';
      const confirm = unique([...dialog.querySelectorAll('.btn-confirm,button,input[type=button]')].filter(element => !element.disabled && /^(确定|确认|是|OK|Confirm)$/i.test((element.innerText || element.value || '').trim())));
      if (confirm && /注销|退出登录|下线|log\s*out|sign\s*out/i.test(message)) {
        window.__buaaControls = {confirm};
        return {state: 'confirm_logout'};
      }
      // Portal messages are not all confirmations; never dismiss a login rejection.
      return {state: 'portal_error', code};
    }
    const logout = button(['#logout','#logout-button','#logout_button','#logout-btn'], ['注销','退出登录','下线','Logout','Log out']);
    // A visible logout control takes precedence, even if stale login fields remain
    // or the control is disabled while the logout request is still in flight.
    if (logout) {
      window.__buaaControls = {logout};
      return {state: 'online', logout_ready: ready(logout)};
    }
    const username = field(['#username','#user_name','input[name=username]','input[autocomplete=username]'], element => element.tagName === 'INPUT' && element.type !== 'password' && !element.disabled);
    const password = field(['#password','#user_password','input[name=password]','input[type=password]'], element => element.tagName === 'INPUT' && element.type === 'password' && !element.disabled);
    const login = button(['#login-account','#login','#login-button','#login_button','#login-btn'], ['登录','登 录','Login','Log in']);
    if (login && (login.disabled || (login.id === 'login-account' && !ready(login)))) return {state: 'loading'};
    window.__buaaControls = {username, password, login};
    return {state: username && password && login ? 'login' : 'waiting'};
  } catch (_) { return {state: 'ambiguous'}; }
})()
