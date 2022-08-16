import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  PropType,
  ref,
  watch,
} from "vue";
import { css } from "@linaria/core";
import { EditorView } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import prettyBytes from "pretty-bytes";

import {
  getResponseBody,
  HTTPResponse,
  ResponseBodyResult,
  getStatusText,
  getLatestResponse,
} from "../../commands/http_request";
import { useSettingStore } from "../../stores/setting";
import { NDivider, NGradientText, NSpace } from "naive-ui";
import { padding } from "../../constants/style";
import { getDefaultExtensions, replaceContent } from "../../helpers/editor";
import { i18nCollection } from "../../i18n";

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

function formatLatency(ms: number) {
  if (ms < 1000) {
    return `${ms.toLocaleString()} ms`;
  }
  return `${(ms / 1000).toFixed(2)} s`;
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
    const statusCode = ref(0);
    const size = ref(-1);
    const latency = ref(0);

    const fillValues = async (resp: HTTPResponse) => {
      // 初始加载时，读取最近的响应
      if (!resp.status) {
        const tmp = getLatestResponse(resp.api);
        if (tmp) {
          resp = tmp;
        }
      }
      statusCode.value = resp.status;
      let body = {
        size: -1,
      } as ResponseBodyResult;
      if (resp.body) {
        body = getResponseBody(resp);
      }
      size.value = body.size;
      latency.value = resp.latency;
      replaceContent(editor, body.data);
    };

    const stop = watch(
      () => props.response,
      (resp) => {
        fillValues(resp);
      }
    );

    const codeEditor = ref<Element>();
    const extensions = getDefaultExtensions({
      isDark: settingStore.isDark,
      readonly: true,
    });
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
      latency,
      statusCode,
      codeEditor,
    };
  },
  render() {
    const { statusCode, size, latency } = this;
    let statusCodeInfo = <span></span>;
    if (statusCode === -1) {
      statusCodeInfo = <span>{i18nCollection("requesting")}</span>;
    } else if (statusCode) {
      statusCodeInfo = (
        <NGradientText type={getStatusType(statusCode)}>
          {statusCode} {getStatusText(statusCode)}
        </NGradientText>
      );
    }

    return (
      <div class={responseClass}>
        <NSpace class="infos">
          {statusCodeInfo}
          {/* 占位 */}
          <span> </span>
          {size >= 0 && prettyBytes(size)}
          {latency > 0 && formatLatency(latency)}
        </NSpace>
        <NDivider />
        <div ref="codeEditor" class="codeEditor"></div>
      </div>
    );
  },
});
