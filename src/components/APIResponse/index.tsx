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
import prettyBytes from "pretty-bytes";

import {
  getResponseBody,
  HTTPResponse,
  ResponseBodyResult,
  getStatusText,
} from "../../commands/http_request";
import { useSettingStore } from "../../stores/setting";
import { NDivider, NGradientText, NSpace } from "naive-ui";
import { padding } from "../../constants/style";

const responseClass = css`
  margin-left: 5px;
  margin-right: 2px;
  .infos {
    height: 46px;
    line-height: 46px;
    padding: 0 ${padding}px;
  }
  .codeEditor {
    position: absolute;
    top: 50px;
    left: 5px;
    right: 2px;
    bottom: 0;
    overflow: auto;
  }
  .n-divider {
    margin: 0;
  }
`;

function getStatusType(statusCode: number) {
  if (statusCode >= 500) {
    return "error";
  }
  if (statusCode >= 400) {
    return "warning";
  }
  return "success";
}

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

    const statusCode = ref(0);
    const size = ref(-1);

    const stop = watch(
      () => props.response,
      (resp) => {
        statusCode.value = resp.status;
        let body = {
          size: -1,
        } as ResponseBodyResult;
        if (resp.body) {
          body = getResponseBody(resp);
        }
        size.value = body.size;
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
      size,
      statusCode,
      codeEditor,
    };
  },
  render() {
    const { statusCode, size } = this;
    const statusCodeInfo = !!statusCode && (
      <NGradientText type={getStatusType(statusCode)}>
        {statusCode} {getStatusText(statusCode)}
      </NGradientText>
    );
    return (
      <div class={responseClass}>
        <NSpace class="infos">
          {statusCodeInfo}
          {/* 占位 */}
          <span> </span>
          {size >= 0 && prettyBytes(size)}
        </NSpace>
        <NDivider />
        <div ref="codeEditor" class="codeEditor"></div>
      </div>
    );
  },
});
