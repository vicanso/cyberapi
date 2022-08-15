import {
  NBadge,
  NButton,
  NDropdown,
  NIcon,
  NTab,
  NTabs,
  useDialog,
  useMessage,
} from "naive-ui";
import { css } from "@linaria/core";
import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  PropType,
  ref,
  watch,
} from "vue";
import { EditorView, ViewUpdate } from "@codemirror/view";
import { EditorState } from "@codemirror/state";

import { HTTPMethod, HTTPRequest } from "../../commands/http_request";
import { useSettingStore } from "../../stores/setting";
import { i18nCollection, i18nCommon } from "../../i18n";
import { CaretDownOutline, CodeSlashOutline } from "@vicons/ionicons5";
import { showError } from "../../helpers/util";
import ExKeyValue, { HandleOption } from "../ExKeyValue";
import { KVParam } from "../../commands/interface";
import { padding } from "../../constants/style";
import { useAPICollectionStore } from "../../stores/api_collection";
import { getDefaultExtensions, replaceContent } from "../../helpers/editor";

enum TabItem {
  Body = "Body",
  Query = "Query",
  Auth = "Auth",
  Header = "Header",
}

enum ContentType {
  JSON = "application/json",
  Form = "application/x-www-form-urlencoded",
  Multipart = "multipart/form-data",
  XML = "application/xml",
  Plain = "text/plain",
}

const tabClass = css`
  .n-tabs-tab__label {
    .n-icon {
      margin-left: 5px;
    }
    .contentType {
      width: 60px;
      text-align: center;
    }
  }
  .badgeTab {
    position: relative;
    .badge {
      position: absolute;
      right: -15px;
      top: 8px;
      .n-badge-sup {
        padding: 0 3px !important;
        border-radius: 3px !important;
      }
    }
  }
  .hidden {
    display: none;
  }
  .format {
    position: fixed;
    bottom: 0;
    .n-icon {
      font-size: 16px;
      font-weight: 900;
      margin-right: 5px;
    }
  }
  .content {
    position: absolute;
    top: 90px;
    left: 0;
    right: 0;
    bottom: 0;
    overflow: auto;
  }
  .keyValue {
    margin: ${padding}px;
  }
`;

function shouldHaveBody(method: string) {
  return [HTTPMethod.POST, HTTPMethod.PUT, HTTPMethod.PATCH].includes(
    method as HTTPMethod
  );
}

function shouldShowEditor(contentType: string) {
  return [ContentType.JSON, ContentType.XML, ContentType.Plain].includes(
    contentType as ContentType
  );
}

function createBadgeTab(params: {
  tab: string;
  value: number;
  activeTab: string;
}) {
  const { value, tab, activeTab } = params;
  const badge =
    value && tab !== activeTab ? (
      <NBadge class="badge" color="grey" value={value} />
    ) : null;
  return (
    <NTab class="badgeTab" name={tab}>
      {tab}
      {badge}
    </NTab>
  );
}

export default defineComponent({
  name: "APISettingParamsReqParams",
  props: {
    id: {
      type: String,
      required: true,
    },
    params: {
      type: Object as PropType<HTTPRequest>,
      required: true,
    },
    onUpdateBody: {
      type: Function as PropType<
        (params: { body: string; contentType: string }) => void
      >,
      required: true,
    },
    onUpdateQuery: {
      type: Function as PropType<(query: KVParam[]) => void>,
      required: true,
    },
    onUpdateHeaders: {
      type: Function as PropType<(headers: KVParam[]) => void>,
      required: true,
    },
  },
  setup(props) {
    const settingStore = useSettingStore();
    const message = useMessage();
    const dialog = useDialog();
    const collecitonStore = useAPICollectionStore();
    const codeEditor = ref<Element>();
    const contentType = ref(props.params.contentType || ContentType.JSON);

    let tab = collecitonStore.getActiveTab(props.id);
    if (!tab) {
      if (shouldHaveBody(props.params.method)) {
        tab = TabItem.Body;
      } else {
        tab = TabItem.Query;
      }
    }
    const activeTab = ref(tab as TabItem);

    let editor: EditorView;
    const destroy = () => {
      if (editor) {
        editor.destroy();
      }
    };
    const handleEditorUpdate = (v: ViewUpdate) => {
      if (v.docChanged && props.onUpdateBody) {
        props.onUpdateBody({
          body: editor.state.doc.toString().trim(),
          contentType: contentType.value,
        });
      }
    };
    const extensions = getDefaultExtensions({
      isDark: settingStore.isDark,
    });
    extensions.push(EditorView.updateListener.of(handleEditorUpdate));
    const initEditor = () => {
      const state = EditorState.create({
        doc: props.params.body,
        extensions,
      });
      editor = new EditorView({
        state,
        parent: codeEditor.value,
      });
    };

    const handleFormat = () => {
      const data = editor.state.doc.toString();
      try {
        const result = JSON.stringify(JSON.parse(data), null, 2);
        if (result !== data) {
          replaceContent(editor, result);
        }
      } catch (err) {
        showError(message, err);
      }
    };
    const handleChangeContentType = (newContentType: string) => {
      // 如果无数据，直接切换
      const changeContentType = () => {
        // 清空
        replaceContent(editor, "");
        if (props.onUpdateBody) {
          props.onUpdateBody({
            body: "",
            contentType: newContentType,
          });
        }
        contentType.value = newContentType;
      };
      if (!props.params.body) {
        changeContentType();
        return;
      }
      dialog.warning({
        title: i18nCollection("changeContentType"),
        content: i18nCollection("changeContentTypeContent"),
        positiveText: i18nCommon("confirm"),
        onPositiveClick: async () => {
          changeContentType();
        },
      });
    };

    const getParamsFromHandleOption = (opt: HandleOption) => {
      const arr = [] as KVParam[];
      opt.params.forEach((item) => {
        const { key, value } = item;
        if (!key && !value) {
          return;
        }
        arr.push({
          key,
          value,
          enabled: item.enabled,
        });
      });
      return arr;
    };

    const handleBodyParams = (opt: HandleOption) => {
      const arr = getParamsFromHandleOption(opt);
      if (props.onUpdateBody) {
        props.onUpdateBody({
          body: JSON.stringify(arr),
          contentType: contentType.value,
        });
      }
    };
    const handleQueryParams = (opt: HandleOption) => {
      const arr = getParamsFromHandleOption(opt);
      if (props.onUpdateQuery) {
        props.onUpdateQuery(arr);
      }
    };

    const handleHeaders = (opt: HandleOption) => {
      const arr = getParamsFromHandleOption(opt);
      if (props.onUpdateHeaders) {
        props.onUpdateHeaders(arr);
      }
    };

    // method变化时要选定对应的tab
    const stop = watch(
      () => props.params.method,
      (method) => {
        if (shouldHaveBody(method)) {
          activeTab.value = TabItem.Body;
        } else {
          activeTab.value = TabItem.Query;
        }
      }
    );
    const handleUpdateActiveTab = async (activeTab: string) => {
      try {
        await collecitonStore.updateActiveTab({
          id: props.id,
          activeTab,
        });
      } catch (err) {
        showError(message, err);
      }
    };

    onMounted(() => {
      initEditor();
    });
    onBeforeUnmount(() => {
      stop();
      destroy();
    });
    return {
      contentType,
      handleBodyParams,
      handleQueryParams,
      handleHeaders,
      handleChangeContentType,
      handleFormat,
      handleUpdateActiveTab,
      activeTab,
      codeEditor,
    };
  },
  render() {
    const { params } = this.$props;
    const { method } = params;
    const { activeTab, contentType } = this;
    const tabs = [TabItem.Query, TabItem.Header, TabItem.Auth];
    if (shouldHaveBody(method)) {
      tabs.unshift(TabItem.Body);
    }
    let activeIndex = tabs.indexOf(activeTab);
    if (activeIndex < 0) {
      activeIndex = 0;
    }

    const contentTypeOptions = [
      {
        label: "JSON",
        key: ContentType.JSON,
      },
      {
        label: "Form",
        key: ContentType.Form,
      },
      {
        label: "Multipart",
        key: ContentType.Multipart,
      },
      {
        label: "XML",
        key: ContentType.XML,
      },
      {
        label: "Plain",
        key: ContentType.Plain,
      },
    ];
    const list = tabs.map((item) => {
      switch (item) {
        case TabItem.Body:
          {
            const label = contentTypeOptions.find(
              (opt) => opt.key === contentType
            );
            if (activeTab !== TabItem.Body) {
              return (
                <NTab name={item}>
                  <div class="contentType">
                    {label?.label}
                    <NIcon>
                      <CaretDownOutline />
                    </NIcon>
                  </div>
                </NTab>
              );
            }
            return (
              <NTab name={item}>
                <NDropdown
                  options={contentTypeOptions}
                  trigger="click"
                  value={contentType}
                  onSelect={(value) => {
                    this.handleChangeContentType(value);
                  }}
                >
                  <div class="contentType">
                    {label?.label}
                    <NIcon>
                      <CaretDownOutline />
                    </NIcon>
                  </div>
                </NDropdown>
              </NTab>
            );
          }
          break;
        case TabItem.Query:
          return createBadgeTab({
            activeTab,
            tab: item,
            value: params.query?.length,
          });
          break;
        case TabItem.Header: {
          return createBadgeTab({
            activeTab,
            tab: item,
            value: params.headers?.length,
          });
        }
        default:
          return <NTab name={item}>{item}</NTab>;
          break;
      }
    });

    let codeEditorClass = "";
    if (activeTab !== TabItem.Body || !shouldShowEditor(contentType)) {
      codeEditorClass = "hidden";
    }
    let showBodyKeyValue = false;
    let keyValues = [];

    switch (activeTab) {
      case TabItem.Body:
        {
          if (!shouldShowEditor(contentType)) {
            showBodyKeyValue = true;
            try {
              keyValues = JSON.parse(this.params.body);
            } catch (err) {
              // 忽略parse出错
              console.error(err);
            }
          }
        }
        break;
      case TabItem.Query:
        {
          keyValues = this.params.query || [];
        }
        break;
      case TabItem.Header: {
        keyValues = this.params.headers || [];
      }
    }

    return (
      <div class={tabClass}>
        <NTabs
          tabsPadding={15}
          key={method}
          type="line"
          defaultValue={tabs[activeIndex]}
          onUpdateValue={(value) => {
            let activeTab = value as string;
            if (shouldHaveBody(method)) {
              if (value === TabItem.Body) {
                activeTab = "";
              }
            } else {
              if (value === TabItem.Query) {
                activeTab = "";
              }
            }
            this.handleUpdateActiveTab(activeTab);
            this.activeTab = value;
          }}
        >
          {list}
        </NTabs>
        <div class="content">
          {/* json, xml, text */}
          <div ref="codeEditor" class={codeEditorClass}></div>
          {activeTab === TabItem.Body && contentType === ContentType.JSON && (
            <NButton
              class="format"
              quaternary
              onClick={() => {
                this.handleFormat();
              }}
            >
              <NIcon>
                <CodeSlashOutline />
              </NIcon>
              {i18nCollection("format")}
            </NButton>
          )}
          {/* body form/multipart */}
          {showBodyKeyValue && (
            <ExKeyValue
              key="form/multipart"
              class="keyValue"
              params={keyValues}
              onHandleParam={(opt) => {
                this.handleBodyParams(opt);
              }}
            />
          )}
          {activeTab === TabItem.Query && (
            <ExKeyValue
              key="query"
              class="keyValue"
              params={keyValues}
              onHandleParam={(opt) => {
                this.handleQueryParams(opt);
              }}
            />
          )}
          {activeTab === TabItem.Header && (
            <ExKeyValue
              key="header"
              class="keyValue"
              params={keyValues}
              onHandleParam={(opt) => {
                this.handleHeaders(opt);
              }}
            />
          )}
        </div>
      </div>
    );
  },
});
