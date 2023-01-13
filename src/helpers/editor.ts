import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { get } from "lodash-es";
import { indentWithTab, toggleComment } from "@codemirror/commands";
import { oneDark } from "@codemirror/theme-one-dark";

export function getDefaultExtensions(params: {
  isDark: boolean;
  readonly?: boolean;
}) {
  const lang = json();
  // TODO 后续确认如何修改
  if (get(lang, "extension[0].data.default[0]")) {
    // eslint-disable-next-line
    // @ts-ignore
    lang.extension[0].data.default[0].commentTokens = {
      line: "//",
    };
  }
  const extensions = [
    basicSetup,
    keymap.of([
      indentWithTab,
      {
        key: "Mod-/",
        run: toggleComment,
      },
    ]),
    lang,
    EditorState.tabSize.of(2),
    EditorView.lineWrapping,
  ];
  if (params.isDark) {
    extensions.push(oneDark);
  }
  if (params.readonly) {
    extensions.push(EditorState.readOnly.of(true));
  }

  return extensions;
}

// 替换内容
export function replaceContent(editor: EditorView, content: string) {
  if (!editor) {
    return;
  }
  const data = editor.state.doc.toString();
  if (content !== data) {
    const trans = editor.state.update({
      changes: {
        from: 0,
        to: data.length,
        insert: content,
      },
    });
    editor.update([trans]);
  }
}
