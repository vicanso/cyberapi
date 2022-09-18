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

FROM alpine 

EXPOSE 7001

# tzdata 安装所有时区配置或可根据需要只添加所需时区

RUN addgroup -g 1000 rust \
  && adduser -u 1000 -G rust -s /bin/sh -D rust \
  && apk add --no-cache ca-certificates tzdata

COPY --from=builder /cberapi-web/target/release/cyberapi-web /usr/local/bin/cyberapi-web
COPY --from=builder /cberapi-web/dist /web
COPY --from=builder /cberapi-web/entrypoint.sh /entrypoint.sh

ENV STATIC_PATH=/web

USER rust

WORKDIR /home/rust

HEALTHCHECK --timeout=10s --interval=10s CMD [ "wget", "http://127.0.0.1:3000/ping", "-q", "-O", "-"]

CMD ["forest"]

ENTRYPOINT ["/entrypoint.sh"]