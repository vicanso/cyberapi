import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  PropType,
  ref,
  StyleValue,
  watch,
} from "vue";
import { css } from "@linaria/core";
import { EditorView } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import prettyBytes from "pretty-bytes";
import { InformationCircleOutline, LinkOutline } from "@vicons/ionicons5";

import {
  HTTPResponse,
  ResponseBodyResult,
  getStatusText,
  getLatestResponse,
  getResponseBody,
  HTTPStats,
} from "../../commands/http_response";
import { useSettingStore } from "../../stores/setting";
import {
  NDescriptions,
  NDescriptionsItem,
  NDivider,
  NGradientText,
  NIcon,
  NPopover,
  NSpace,
  useMessage,
} from "naive-ui";
import { padding } from "../../constants/style";
import { getDefaultExtensions, replaceContent } from "../../helpers/editor";
import { i18nCollection } from "../../i18n";
import { convertRequestToCURL, HTTPRequest } from "../../commands/http_request";
import { showError, writeTextToClipboard } from "../../helpers/util";
import ExPreview, { isSupportPreview } from "../ExPreview";
import { useRoute } from "vue-router";

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
  .info {
    cursor: pointer;
    float: left;
    margin-top: 15px;
    font-size: 16px;
  }
  .hidden {
    display: none;
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

const showCurlLimitSize = 2 * 1024;
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
    const route = useRoute();
    const message = useMessage();
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
    const apiID = ref("");
    const stats = ref({} as HTTPStats);
    const collection = route.query.collection as string;

    const previewMode = ref(false);
    const previewData = ref({
      contentType: "",
      data: "",
    });

    let req: HTTPRequest;

    const reqExists = ref(false);
    const curl = ref("");

    const fillValues = async (resp: HTTPResponse) => {
      // 初始加载时，读取最近的响应
      if (!resp.status) {
        const tmp = await getLatestResponse(resp.api);
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

      let contentType = "";
      resp.headers?.forEach((values, key) => {
        if (key.toLowerCase() === "content-type") {
          contentType = values[0];
        }
      });

      if (isSupportPreview(contentType)) {
        previewMode.value = true;
        previewData.value = {
          contentType,
          data: body.data,
        };
      } else {
        previewMode.value = false;
      }
      size.value = body.size;
      latency.value = resp.latency;
      apiID.value = resp.api;
      req = resp.req;
      if (!req) {
        reqExists.value = false;
      } else {
        reqExists.value = true;
      }
      curl.value = "";
      stats.value = resp.stats;
      if (!previewMode.value) {
        replaceContent(editor, body.data);
      }
    };

    const handleToCURL = async () => {
      if (!req || curl.value) {
        return;
      }
      try {
        const value = await convertRequestToCURL(collection, req);
        if (value.length > showCurlLimitSize) {
          await writeTextToClipboard(value);
        }
        curl.value = value;
      } catch (err) {
        showError(message, err);
      }
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
      curl,
      reqExists,
      size,
      stats,
      latency,
      statusCode,
      apiID,
      previewMode,
      previewData,
      codeEditor,
      handleToCURL,
    };
  },
  render() {
    const {
      statusCode,
      size,
      latency,
      apiID,
      curl,
      reqExists,
      stats,
      previewMode,
      previewData,
    } = this;
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

    const apiIDSlots = {
      trigger: () => (
        <NIcon class="info">
          <InformationCircleOutline />
        </NIcon>
      ),
    };

    const curlSlots = {
      trigger: () => (
        <NIcon class="info">
          <LinkOutline />
        </NIcon>
      ),
    };
    const isTooLarge = curl.length > showCurlLimitSize;
    let curlText = i18nCollection("curlTooLargeTips");
    if (!isTooLarge) {
      curlText = curl;
    }
    const curlStyle: StyleValue = isTooLarge
      ? {}
      : {
          width: "400px",
          "word-break": "break-all",
          "word-wrap": "break-word",
        };

    const descriptionItemOptions = [
      {
        label: i18nCollection("apiID"),
        key: "apiID",
        value: apiID,
      },
    ];
    if (stats?.remoteAddr) {
      descriptionItemOptions.push({
        label: i18nCollection("remoteAddr"),
        key: "remoteAddr",
        value: stats.remoteAddr,
      });
    }

    const descriptionItems = descriptionItemOptions.map((item) => {
      return (
        <NDescriptionsItem label={item.label} key={item.key} span={3}>
          {item.value}
        </NDescriptionsItem>
      );
    });

    const codeEditorCls = {
      codeEditor: true,
      hidden: false,
    };
    if (previewMode) {
      codeEditorCls.hidden = true;
    }

    return (
      <div class={responseClass}>
        <NSpace class="infos">
          {apiID && (
            <NPopover v-slots={apiIDSlots} trigger="click" placement="bottom">
              <NDescriptions labelPlacement="left">
                {descriptionItems}
              </NDescriptions>
            </NPopover>
          )}
          {reqExists && (
            <NPopover
              v-slots={curlSlots}
              trigger="click"
              placement="bottom"
              onUpdateShow={(value) => {
                if (value) {
                  this.handleToCURL();
                }
              }}
            >
              <div style={curlStyle}>{curlText}</div>
            </NPopover>
          )}
          {statusCodeInfo}
          {/* 占位 */}
          <span> </span>
          {size >= 0 && prettyBytes(size)}
          {latency > 0 && formatLatency(latency)}
        </NSpace>
        <NDivider />
        <div ref="codeEditor" class={codeEditorCls}></div>
        {codeEditorCls.hidden && (
          <ExPreview
            contentType={previewData.contentType}
            data={previewData.data}
          />
        )}
      </div>
    );
  },
});
