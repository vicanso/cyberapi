# Cyber API


CyberAPI是基于`tauri`[]开发的跨平台API客户端，用于开发测试HTTP的接口。


## 创建项目

首次启动时，需要先创建项目，建议将不同的项目来创建，同一项目可共用环境变量的配置。

<p align="center">
    <img src="./asset/home.png" alt="home">
</p>

### 创建项目的环境变量

环境变量主要用于定义HTTP请求的host等场景，用于方便快捷的切换请求对接的环境。

<p align="center">
    <img src="./asset/env-select.png" alt="env-select">
</p>

tiny配置了两个环境的ENV设置，其中`http://tiny.npmtrend.com`未生效(复选框未勾选)，如果需要切换不同的环境时，则选择勾选不同的配置生效即可，需要注意不要同时选择相同的环境变量生效。

<p align="center">
    <img src="./asset/env-editor.png" alt="env-editor">
</p>

### 创建目录与请求

创建请求之前，建议按功能来创建不同的分组，例如创建用户相关的一个分组：

<p align="center">
    <img src="./asset/create-folder.png" alt="create-folder">
</p>

在创建分组之后，则可以在该分组下创建对应的请求：

<p align="center">
    <img src="./asset/create-request.png" alt="create-request">
</p>


在创建请求之后，则可以选择请求使用的env(自动添加至请求url中)，对应的HTTP Method以及输入URL。对于POST类请求的body部分，则可以选择对应的数据类型，如选择了json数据，填写对应的参数，图中的`{{md5(123123)}}`为函数形式，会在请求时执行此函数，填充对应的数据，后续会专门介绍有哪些自定义的函数：

<p align="center">
    <img src="./asset/create-request-detail.png" alt="create-request-detail">
</p>


配置完成后，点击发送则发送该请求，获取到响应后则展示如下图。第一个图标点击时会展示该请求的ID(后续可用于其它请求指定获取该请求的响应时使用)，第二个图标点击会会展示此请求对应的`curl`。

<p align="center">
    <img src="./asset/request-result.png" alt="request-result">
</p>



# 查询证件列表

```
security find-identity -v -p codesigning
```
# 校验IMG

校验IMG是否使用证件签名：
```
spctl -a -v src-tauri/target/release/bundle/dmg/cyberapi_0.1.0_aarch64.dmg
```