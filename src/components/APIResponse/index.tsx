import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  PropType,
  ref,
  watch,
} from "vue";
import { css } from "@linaria/core";
import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";
import { indentWithTab } from "@codemirror/commands";

import {
  getResponseBody,
  HTTPResponse,
  ResponseBodyResult,
} from "../../commands/http_request";
import { useSettingStore } from "../../stores/setting";

const responseClass = css`
  margin-left: 3px;
`;

export default defineComponent({
  name: "APIResponse",
  props: {
    response: {
      type: Object as PropType<HTTPResponse>,
      default: () => {
        return {
          status: -1,
          headers: new Map<string, string[]>(),
          body: "",
        };
      },
    },
  },
  setup(props) {
    const settingStore = useSettingStore();
    let editor: EditorView;
    const destroy = () => {
      if (editor) {
        editor.destroy();
      }
    };
    const replaceContent = (result: string) => {
      const data = editor.state.doc.toString();
      if (result !== data) {
        const trans = editor.state.update({
          changes: {
            from: 0,
            to: data.length,
            insert: result,
          },
        });
        editor.update([trans]);
      }
    };

    const stop = watch(
      () => props.response,
      (resp) => {
        let body = {} as ResponseBodyResult;
        if (resp.body) {
          body = getResponseBody(resp);
        }
        replaceContent(body.data);
      }
    );

    const codeEditor = ref<Element>();
    const extensions = [
      basicSetup,
      keymap.of([indentWithTab]),
      json(),
      EditorState.readOnly.of(true),
    ];
    if (settingStore.isDark) {
      extensions.push(oneDark);
    }
    const initEditor = () => {
      const state = EditorState.create({
        extensions,
      });
      editor = new EditorView({
        state,
        parent: codeEditor.value,
      });
      editor.dispatch({});
    };

    onMounted(() => {
      initEditor();
    });
    onBeforeUnmount(() => {
      destroy();
      stop();
    });

    return {
      codeEditor,
    };
  },
  render() {
    return (
      <div class={responseClass}>
        <div ref="codeEditor"></div>
      </div>
    );
  },
});
