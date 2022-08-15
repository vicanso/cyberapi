import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { indentWithTab } from "@codemirror/commands";
import { oneDark } from "@codemirror/theme-one-dark";

export function getDefaultExtensions(params: {
  isDark: boolean;
  readonly?: boolean;
}) {
  const extensions = [
    basicSetup,
    keymap.of([indentWithTab]),
    json(),
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
