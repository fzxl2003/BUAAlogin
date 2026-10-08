"""Interactive single login example (password is not stored in source)."""
import getpass
from BUAASrunLogin.LoginManager import LoginManager

if __name__ == '__main__':
    result = LoginManager().login(input('校园网账号: ').strip(), getpass.getpass('校园网密码: '))
    print(result)
