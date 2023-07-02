import * as words from "shellwords";
import { forEach } from "lodash-es";
import { isJSON } from "./util";

export default function(s) {
  const params = parse(s);
  let contentType = "";
  const headers = [];
  forEach(params.header, (value, key) => {
    if (key.toLowerCase() === "content-type") {
      contentType = value;
      return;
    }
    headers.push({
      key,
      value,
      enabled: true,
    });
  });
  if (!contentType && isJSON(params.body)) {
    contentType = "application/json";
  }
  const index = contentType.indexOf(";");
  if (index !== -1) {
    contentType = contentType.substring(0, index);
  }
  const result = new URL(params.url);
  const query = [];
  result.searchParams.forEach((value, key) => {
    query.push({
      key,
      value,
      enabled: true,
    });
  });
  return {
    method: params.method,
    uri: `${result.origin}${result.pathname}`,
    body: params.body,
    contentType,
    headers,
    query,
  }
}

function parse(s) {
  if (0 != s.indexOf('curl ')) return
  var args = rewrite(words.split(s))
  var out = { method: 'GET', header: {} }
  var state = ''

  args.forEach(function (arg) {
    switch (true) {
      case isURL(arg):
        out.url = arg
        break;

      case arg == '-A' || arg == '--user-agent':
        state = 'user-agent'
        break;

      case arg == '-H' || arg == '--header':
        state = 'header'
        break;

      case arg == '-d' || arg == '--data' || arg == '--data-ascii':
        state = 'data'
        break;

      case arg == '-u' || arg == '--user':
        state = 'user'
        break;

      case arg == '-I' || arg == '--head':
        out.method = 'HEAD'
        break;

      case arg == '-X' || arg == '--request':
        state = 'method'
        break;

      case arg == '-b' || arg == '--cookie':
        state = 'cookie'
        break;

      case arg == '--compressed':
        out.header['Accept-Encoding'] = out.header['Accept-Encoding'] || 'deflate, gzip'
        break;

      case !!arg:
        switch (state) {
          case 'header':
            var field = parseField(arg)
            out.header[field[0]] = field[1]
            state = ''
            break;
          case 'user-agent':
            out.header['User-Agent'] = arg
            state = ''
            break;
          case 'data':
            if (out.method == 'GET' || out.method == 'HEAD') out.method = 'POST'
            out.header['Content-Type'] = out.header['Content-Type'] || 'application/x-www-form-urlencoded'
            out.body = out.body
              ? out.body + '&' + arg
              : arg
            state = ''
            break;
          case 'user':
            out.header['Authorization'] = 'Basic ' + window.btoa(arg)
            state = ''
            break;
          case 'method':
            out.method = arg
            state = ''
            break;
          case 'cookie':
            out.header['Set-Cookie'] = arg
            state = ''
            break;
        }
        break;
    }
  })

  return out
}

/**
 * Rewrite args for special cases such as -XPUT.
 */

function rewrite(args) {
  return args.reduce(function (args, a) {
    if (0 == a.indexOf('-X')) {
      args.push('-X')
      args.push(a.slice(2))
    } else {
      args.push(a)
    }

    return args
  }, [])
}

/**
 * Parse header field.
 */

function parseField(s) {
  return s.split(/: ?(.+)/)
}

/**
 * Check if `s` looks like a url.
 */

function isURL(s) {
  // TODO: others at some point
  return /^https?:\/\//.test(s)
}