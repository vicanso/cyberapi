FROM node:18-alpine as webbuilder

COPY . /cberapi-web
RUN cd /cberapi-web/web \
  && npm i \
  && npm run build \
  && rm -rf node_module

FROM rust:alpine as builder

COPY --from=webbuilder /cberapi-web /cberapi-web

RUN apk update \
  && apk add git make musl-dev \
  && cd /cberapi-web \
  && rm -rf dist \
  && cp -rf web/dist dist \
  && cp -rf docs dist/ \
  && cp assets/* dist/assets \
  && cargo build --release