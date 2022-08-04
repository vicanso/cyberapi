const metaKey = "⌘";
const shiftKey = "⇧";

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
      case shiftKey: {
        if (!e.shiftKey) {
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

export function hotKeyCreateFolder() {
  return `${shiftKey} ${metaKey} N`;
}

export function hotKeyMatchCreateFolder(e: KeyboardEvent) {
  return match(hotKeyCreateFolder(), e);
}
