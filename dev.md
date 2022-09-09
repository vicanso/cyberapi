# 查询证件列表

```
security find-identity -v -p codesigning
```
# 校验IMG

校验IMG是否使用证件签名：
```
spctl -a -v src-tauri/target/release/bundle/dmg/cyberapi_0.1.0_aarch64.dmg
```