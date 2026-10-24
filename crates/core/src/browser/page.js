(() => {
  const visible = element => {
    const box = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return box.width > 0 && box.height > 0 && style.visibility !== 'hidden' && style.display !== 'none' && !element.disabled;
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
  try {
    if ([...document.querySelectorAll('input')].some(element => visible(element) && /captcha|verify.?code|验证码/i.test([element.id, element.name, element.placeholder].join(' ')))) return {state: 'needs_user'};
    const notices = [...document.querySelectorAll('[role=alert],.layui-layer-content,.error,.error-msg,.error-message,#error,#error_msg')].filter(visible).map(element => element.innerText || '').join(' ');
    if (/密码错误|账号禁用|帐号禁用|余额不足|欠费|验证码|password.error|captcha/i.test(notices)) return {state: 'needs_user'};
    const logout = button(['#logout','#logout-button','#logout_button','#logout-btn'], ['注销','退出登录','下线','Logout','Log out']);
    const username = field(['#username','#user_name','input[name=username]','input[autocomplete=username]'], element => element.tagName === 'INPUT' && element.type !== 'password');
    const password = field(['#password','#user_password','input[name=password]','input[type=password]'], element => element.tagName === 'INPUT' && element.type === 'password');
    const login = button(['#login','#login-button','#login_button','#login-btn'], ['登录','登 录','Login','Log in']);
    const confirm = button([], ['确定','确认','是','OK','Confirm']);
    window.__buaaControls = {logout, username, password, login, confirm};
    return {state: logout && !password ? 'online' : username && password && login ? 'login' : confirm ? 'confirm' : 'waiting'};
  } catch (_) { return {state: 'ambiguous'}; }
})()
