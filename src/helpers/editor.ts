import { editor } from "monaco-editor/esm/vs/editor/editor.api";

export function createEditor(params: {
  isDark: boolean;
  readonly?: boolean;
  dom: HTMLElement;
}) {
  // * The current out-of-the-box available themes are: 'vs' (default), 'vs-dark', 'hc-black', 'hc-light.
  const e = editor.create(params.dom, {
    readOnly: params.readonly || false,
    language: "json",
    theme: params.isDark ? "vs-dark" : "vs",
    automaticLayout: true,
  });
  e.updateOptions({
    fontSize: 14,
    lineNumbersMinChars: 4,
    wordWrap: "on",
  });
  return e;
}

// 替换内容
export function replaceContent(
  editor: editor.IStandaloneCodeEditor | null,
  content: string
) {
  if (!editor) {
    return;
  }
  editor.setValue(content || "");
}
