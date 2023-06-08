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
import prettyBytes from "pretty-bytes";
import {
  BowlingBallOutline,
  InformationCircleOutline,
  LinkOutline,
} from "@vicons/ionicons5";
import { editor } from "monaco-editor/esm/vs/editor/editor.api";

import {
  HTTPResponse,
  ResponseBodyResult,
  getLatestResponse,
  getResponseBody,
  HTTPStats,
} from "../../commands/http_response";
import { useSettingStore } from "../../stores/setting";
import {
  NDescriptions,
  NDescriptionsItem,
  NDivider,
  NIcon,
  NPopover,
  NSpace,
  useMessage,
} from "naive-ui";
import { useRoute } from "vue-router";
import { padding } from "../../constants/style";
import { replaceContent, createEditor } from "../../helpers/editor";
import { i18nCollection, i18nCommon } from "../../i18n";
import { convertRequestToCURL, HTTPRequest } from "../../commands/http_request";
import {
  convertHTTPHeaderName,
  showError,
  writeFileToDownload,
  writeTextToClipboard,
  formatLatency,
} from "../../helpers/util";
import ExPreview, { isSupportPreview } from "../ExPreview";
import ExTimer from "../ExTimer";
import APIResponseList from "./list";
import APIResponseStatusText from "./status_text";
import { toUint8Array } from "js-base64";
import { useCookieStore } from "../../stores/cookie";
import { useAPISettingStore } from "../../stores/api_setting";

const responseClass = css`
  margin-left: 5px;
  margin-right: 2px;
  .infos {
    height: 48px;
    line-height: 48px;
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
    padding: 0 2px;
    font-size: 20px;
    font-weight: 600;
  }
  .cookie {
    margin-top: 16px;
  }
  .header {
    padding: 0 5px;
    cursor: pointer;
    font-size: 16px;
  }
  .hidden {
    display: none;
  }
  .responseList {
    float: right;
  }
`;

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
    const cookieStore = useCookieStore();
    const apiSettingStore = useAPISettingStore();
    let editorIns: editor.IStandaloneCodeEditor | null;
    const destroy = () => {
      if (editorIns) {
        editorIns = null;
      }
    };
    const statusCode = ref(0);
    const size = ref(-1);
    const originalSize = ref(-1);
    const latency = ref(0);
    const apiID = ref("");
    const stats = ref({} as HTTPStats);
    const headers = ref(new Map<string, string[]>());
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
      let isFromCache = false;
      if (!resp.status) {
        const tmp = await getLatestResponse(resp.api);
        isFromCache = true;
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
      let filename = "";
      resp.headers?.forEach((values, key) => {
        const k = key.toLowerCase();
        switch (k) {
          case "content-type":
            contentType = values[0];
            break;
          case "content-disposition":
            {
              const reg = /filename="([\s\S]*?)"/;
              const result = reg.exec(values[0]);
              if (result?.length === 2) {
                filename = result[1];
              }
            }
            break;
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
      originalSize.value = resp.bodySize;
      size.value = body.size;
      latency.value = resp.latency;
      apiID.value = resp.api;
      headers.value = resp.headers;
      req = resp.req;
      if (!req) {
        reqExists.value = false;
      } else {
        reqExists.value = true;
      }
      curl.value = "";
      stats.value = resp.stats;
      if (filename) {
        if (!isFromCache) {
          writeFileToDownload(filename, toUint8Array(body.data))
            .then(() => {
              message.info(i18nCommon("saveToDownloadSuccess"));
            })
            .catch((err) => {
              showError(message, err);
            });
        }
      } else if (!previewMode.value) {
        replaceContent(editorIns, body.data);
      }
    };

    const handleToCURL = async () => {
      if (!req || curl.value) {
        return;
      }
      try {
        // 由于cookie会一直更新，因此此时再拉取
        await cookieStore.fetch();
        // req 对象为未调用uri部分，需要先调整
        apiSettingStore.fillValues(req);
        const value = await convertRequestToCURL(
          collection,
          req,
          cookieStore.cookies
        );
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

    const codeEditor = ref<HTMLElement>();
    const initEditor = () => {
      if (codeEditor.value) {
        editorIns = createEditor({
          readonly: true,
          dom: codeEditor.value,
          isDark: settingStore.isDark,
        });
      }
    };

    onMounted(() => {
      initEditor();
      if (props.response.api) {
        fillValues(props.response);
      }
    });
    onBeforeUnmount(() => {
      destroy();
      stop();
    });

    return {
      curl,
      reqExists,
      headers,
      size,
      originalSize,
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
      originalSize,
      latency,
      apiID,
      curl,
      headers,
      reqExists,
      stats,
      previewMode,
      previewData,
    } = this;
    let statusCodeInfo = <span></span>;
    if (statusCode === -1) {
      statusCodeInfo = (
        <span>
          {i18nCollection("requesting")} <ExTimer />
        </span>
      );
    } else if (statusCode) {
      statusCodeInfo = <APIResponseStatusText statusCode={statusCode} />;
    }

    const apiIDSlots = {
      trigger: () => (
        <NIcon class="info" size={20}>
          <InformationCircleOutline />
        </NIcon>
      ),
    };

    const headerSlots = {
      trigger: () => <span class="header">H</span>,
    };

    const curlSlots = {
      trigger: () => (
        <NIcon class="info" size={20}>
          <LinkOutline />
        </NIcon>
      ),
    };

    const cookieSlots = {
      trigger: () => (
        <NIcon class="info cookie" size={17}>
          <BowlingBallOutline />
        </NIcon>
      ),
    };
    const isTooLarge = curl.length > showCurlLimitSize;
    let curlText = i18nCollection("curlTooLargeTips");
    if (!isTooLarge) {
      curlText = curl;
    }
    if (curl.length === 0) {
      curlText = i18nCollection("curlGenerateFail");
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
    if (stats?.cipher) {
      descriptionItemOptions.push({
        label: i18nCollection("cipher"),
        key: "cipher",
        value: `${stats.cipher}`,
      });
    }

    if (stats?.dnsLookup) {
      descriptionItemOptions.push({
        label: i18nCollection("dns"),
        key: "dns",
        value: formatLatency(stats.dnsLookup),
      });
    }
    if (stats) {
      descriptionItemOptions.push({
        label: i18nCollection("tcp"),
        key: "tcp",
        value: formatLatency(stats.tcp),
      });
      if (stats.isHttps) {
        descriptionItemOptions.push({
          label: i18nCollection("tls"),
          key: "tls",
          value: formatLatency(stats.tls),
        });
      }
      descriptionItemOptions.push(
        {
          label: i18nCollection("send"),
          key: "send",
          value: formatLatency(stats.send),
        },
        {
          label: i18nCollection("serverProcessing"),
          key: "serverProcessing",
          value: formatLatency(stats.serverProcessing),
        },
        {
          label: i18nCollection("contentTransfer"),
          key: "contentTransfer",
          value: formatLatency(stats.contentTransfer),
        }
      );
    }

    const headerDescriptionItems: JSX.Element[] = [];
    const cookieDescriptionItems: JSX.Element[] = [];
    const setCookieKey = "set-cookie";
    if (headers && headers.size !== 0) {
      headers.forEach((values, key) => {
        values.forEach((value, index) => {
          if (key === setCookieKey) {
            const cookie = value.split(";")[0];
            if (!cookie) {
              return;
            }
            const arr = cookie.split("=");
            cookieDescriptionItems.push(
              <NDescriptionsItem label={arr[0]} key={`${arr[0]}`} span={3}>
                {arr.slice(1).join("=")}
              </NDescriptionsItem>
            );
            return;
          }
          headerDescriptionItems.push(
            <NDescriptionsItem
              label={convertHTTPHeaderName(key)}
              key={`${key}-${index}`}
              span={3}
            >
              {value}
            </NDescriptionsItem>
          );
        });
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

    const popupContentStyle: StyleValue = {
      maxWidth: "600px",
      wordBreak: "break-all",
      wordWrap: "break-word",
    };

    const sizeDesc: string[] = [];
    if (size > 0) {
      sizeDesc.push(prettyBytes(size));
      if (originalSize > 0 && Math.abs(size - originalSize) > 100) {
        const percent = `${Math.ceil((originalSize * 100) / size)}%`;
        sizeDesc.push(`${prettyBytes(originalSize)}(${percent})`);
      }
    }

    return (
      <div class={responseClass}>
        {apiID && (
          <div class="responseList">
            {" "}
            <APIResponseList id={apiID} />{" "}
          </div>
        )}
        <NSpace class="infos">
          {apiID && (
            <NPopover v-slots={apiIDSlots} trigger="click" placement="bottom">
              <NDescriptions labelPlacement="left" size="small">
                {descriptionItems}
              </NDescriptions>
            </NPopover>
          )}
          {cookieDescriptionItems.length !== 0 && (
            <NPopover
              v-slots={cookieSlots}
              trigger="click"
              placement="bottom"
              contentStyle={popupContentStyle}
            >
              <NDescriptions labelPlacement="left" size="small">
                {cookieDescriptionItems}
              </NDescriptions>
            </NPopover>
          )}
          {headerDescriptionItems.length !== 0 && (
            <NPopover v-slots={headerSlots} trigger="click" placement="bottom">
              <NDescriptions
                contentStyle={popupContentStyle}
                labelPlacement="left"
                size="small"
              >
                {headerDescriptionItems}
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
          {sizeDesc.length !== 0 && sizeDesc.join(" / ")}
          {/* 占位 */}
          <span> </span>
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
