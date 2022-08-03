const metaKey = "⌘";

function match(hotKey: string, e: KeyboardEvent) {
  let matched = true;
  hotKey.split(" ").forEach((key) => {
    switch (key) {
      case metaKey: {
        if (!e.metaKey) {
          matched = false;
        }
        break;
      }
      default: {
        if (key.toLowerCase() !== e.key) {
          matched = false;
        }
        break;
      }
    }
  });
  return matched;
}

export function hotKeyCreateHTTPSetting() {
  return `${metaKey} N`;
}

export function hotKeyMatchCreateHTTPSetting(e: KeyboardEvent) {
  return match(hotKeyCreateHTTPSetting(), e);
}
