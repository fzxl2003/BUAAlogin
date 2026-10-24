use crate::{cancellation::Cancellation, transport::AuthError};
use serde_json::{json, Value};
use std::{io::ErrorKind, net::TcpStream, time::Duration};
use tungstenite::{Message, WebSocket};

pub(super) struct Cdp {
    socket: WebSocket<TcpStream>,
    next_id: u64,
    cancel: Cancellation,
    origin: String,
}
impl Cdp {
    pub fn connect(
        address: std::net::SocketAddr,
        path: &str,
        cancel: Cancellation,
        origin: String,
    ) -> Result<Self, AuthError> {
        cancel.check()?;
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|_| "无法连接后台浏览器")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "调试连接配置失败")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(|_| "调试连接配置失败")?;
        let (mut socket, _) = tungstenite::client(format!("ws://{address}{path}"), stream)
            .map_err(|_| "后台浏览器握手失败")?;
        cancel.check()?;
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|_| "调试连接配置失败")?;
        Ok(Self {
            socket,
            next_id: 0,
            cancel,
            origin,
        })
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, AuthError> {
        self.cancel.check()?;
        self.next_id += 1;
        let id = self.next_id;
        self.socket
            .send(Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .map_err(|_| "后台浏览器命令发送失败")?;
        loop {
            self.cancel.check()?;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| "后台浏览器应答格式错误")?;
                    if value["method"] == "Page.javascriptDialogOpening" {
                        let message = value["params"]["message"].as_str().unwrap_or("");
                        if [
                            "密码错误",
                            "禁用",
                            "欠费",
                            "验证码",
                            "password_error",
                            "captcha",
                        ]
                        .iter()
                        .any(|part| message.contains(part))
                        {
                            return Err(AuthError::needs_user(
                                "认证页面需要处理：请检查账号、密码、余额或验证码",
                            ));
                        }
                        if value["params"]["type"] != "confirm"
                            || !["注销", "退出登录", "下线", "logout", "log out", "sign out"]
                                .iter()
                                .any(|part| message.to_lowercase().contains(part))
                        {
                            return Err(
                                "认证页面出现提示，未确认登录成功；请在门户查看详细信息".into()
                            );
                        }
                        self.next_id += 1;
                        self.socket.send(Message::Text(json!({"id":self.next_id,"method":"Page.handleJavaScriptDialog","params":{"accept":true}}).to_string().into())).map_err(|_| "认证确认弹窗处理失败")?;
                    }
                    if value["id"].as_u64() == Some(id) {
                        if value.get("error").is_some() {
                            return Err("后台浏览器操作失败".into());
                        }
                        return Ok(value["result"].clone());
                    }
                }
                Ok(Message::Close(_)) => return Err("后台浏览器已关闭".into()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("后台浏览器连接已中断".into()),
            }
        }
    }
    pub fn evaluate(&mut self, expression: &str) -> Result<Value, AuthError> {
        let expression = format!("location.href === 'about:blank' ? ({{state:'waiting'}}) : location.origin === {} ? ({expression}) : ({{state:'wrong_origin'}})", json!(self.origin));
        let value = self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue":true}),
        )?;
        if value.get("exceptionDetails").is_some() {
            return Err("认证页面脚本执行失败".into());
        }
        let result = value["result"]["value"].clone();
        if result["state"] == "wrong_origin" {
            return Err("认证页面地址不被允许".into());
        }
        Ok(result)
    }
    pub fn click(&mut self, control: &str) -> Result<(), AuthError> {
        let position = self.evaluate(&format!("(() => {{ const element = window.__buaaControls?.[{control}]; if (!element || !element.isConnected || element.disabled) return null; element.scrollIntoView({{block:'center'}}); const box = element.getBoundingClientRect(); if (box.width <= 0 || box.height <= 0) return null; const x = box.x + box.width/2, y = box.y + box.height/2; const hit = document.elementFromPoint(x,y); return hit && (hit === element || element.contains(hit)) ? {{x,y}} : null; }})()", control=json!(control)))?;
        let x = position["x"].as_f64().ok_or("认证控件不可点击")?;
        let y = position["y"].as_f64().ok_or("认证控件不可点击")?;
        self.call(
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}),
        )?;
        self.call(
            "Input.dispatchMouseEvent",
            json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}),
        )?;
        Ok(())
    }
    pub fn fill(&mut self, control: &str, text: &str) -> Result<(), AuthError> {
        self.click(control)?;
        let focused = self.evaluate(&format!(
            "document.activeElement === window.__buaaControls?.[{}]",
            json!(control)
        ))?;
        if focused != true {
            return Err("认证输入框未获得焦点".into());
        }
        self.call("Input.dispatchKeyEvent", json!({"type":"keyDown","key":"a","code":"KeyA","modifiers": if cfg!(target_os="macos") {4} else {2}}))?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({"type":"keyUp","key":"a","code":"KeyA"}),
        )?;
        self.call("Input.insertText", json!({"text":text}))?;
        let filled = self.evaluate(&format!(
            "document.activeElement === window.__buaaControls?.[{}] && document.activeElement.value === {}",
            json!(control), json!(text)
        ))?;
        if filled != true {
            return Err("认证输入框填写失败".into());
        }
        Ok(())
    }
}
