# Cloud Deck macOS 安装

v0.1.3 为 Apple Silicon 提供 `CloudDeck-0.1.3-macos-arm64.dmg`。应用采用 ad-hoc 签名，尚未经过 Apple 公证。

1. 从 [GitHub Releases](https://github.com/rezoch340/cloud-deck/releases) 下载 DMG 和同版本 `SHA256SUMS.txt`。运行 `shasum -a 256 <下载文件的路径>`，核对清单中同名文件的 SHA-256。
2. 退出旧 Cloud Deck，双击 DMG，将 `Cloud Deck.app` 拖到「应用程序」，然后推出磁盘映像。使用 ZIP 时，解压后将应用拖到同一目录。
3. 在终端先验签，通过后仅清除这个应用的下载隔离标记，再打开：

```sh
codesign --verify --deep --strict --verbose=2 "/Applications/Cloud Deck.app" &&
xattr -dr com.apple.quarantine "/Applications/Cloud Deck.app" &&
open "/Applications/Cloud Deck.app"
```

若应用没有 quarantine 属性，可在验签成功后直接打开。不要对应用目录以外的路径递归清除属性，也不需要关闭系统 Gatekeeper。

ad-hoc 签名不保证免除 macOS 安全提示。也可以使用系统“隐私与安全性”中的“仍要打开”。从个人开发签名切换到此包、或安装后续 ad-hoc 版本时，macOS 可能再次询问钥匙串访问权限。
