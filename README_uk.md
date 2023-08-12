# Cyber API


[![test library](https://img.shields.io/github/workflow/status/vicanso/cyberapi/test?label=test)](https://github.com/vicanso/cyberapi/actions?query=workflow%3A%22test%22)
[![License](https://img.shields.io/badge/License-Apache%202-green.svg)](https://github.com/vicanso/cyberapi)
[![donwnload](https://img.shields.io/github/downloads/vicanso/cyberapi/total?label=Downloads&logoColor=fff&logo=GitHub)](https://github.com/vicanso/cyberapi/releases)


<p align="center">
    <img src="./cyberapi.png" alt="cyberapi" width="128">
</p>

<h3 align="center">
<a href="https://github.com/vicanso/cyberapi">CyberAPI</a> це інструмент для запитів до API оснований на <a href="https://github.com/tauri-apps/tauri">tauri</a>.
</h3>

[English](./README.md)|[简体中文](./README_zh.md)|Українська
## Features

- Підтримка платформ macos, windows і linux, інсталяційний пакет менше 10 МБ
- Тисячі інтерфейсів для одного проекту відкриваються за лічені секунди, з низьким споживанням пам'яті
- Підтримка темної / світлої теми та китайської / англійської / української мови
- Проста експлуатація та конфігурація
- Підтримка імпорту конфігурації з postman, insonmia або swagger.
- Конфігурацію можна експортувати за інтерфейсом, функцією та проектом, що зручно для спільного використання
- Підтримка багатьох користувацьких функцій


<p align="center">
    <img src="./asset/cyberapi.png" alt="cyberapi">
</p>


## Installation

Інсталятор можна завантажити з [release](https://github.com/vicanso/cyberapi/releases), включаючи версії для Windows, macos і linux.

У Windows може знадобитися встановити webview2.
## Development


Проект залежить від rust та Nodejs. Якщо ви хочете самостійно компілювати або брати участь у розробці, ви можете спочатку встановити залежності tauri, звернувшись до відповідних документів [тут](https://tauri.app/v1/guides/getting-started/prerequisites), а потім :

```shell
yarn
```

Встановити tauri-cli:

```shell
cargo install tauri-cli
```

При запуску в режимі браузера дані є імітацією:

```shell
yarn dev
```

Запустити в режимі додатку: 

```shell
make dev
```

Зібрати релізний пакет:

```shell
make build
```



