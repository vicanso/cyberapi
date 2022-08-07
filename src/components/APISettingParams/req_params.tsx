import { NButton, NIcon, NTab, NTabs, useMessage } from "naive-ui";
import { css } from "@linaria/core";
import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  PropType,
  ref,
  watch,
} from "vue";
import { basicSetup } from "codemirror";
import { EditorView, keymap, ViewUpdate } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";
import { indentWithTab } from "@codemirror/commands";

import { HTTPMethod, HTTPRequest } from "../../commands/http_request";
import { useSettingStore } from "../../stores/setting";
import { i18nCollection } from "../../i18n";
import { CodeSlashOutline } from "@vicons/ionicons5";
import { showError } from "../../helpers/util";

enum TabItem {
  Body = "Body",
  Query = "Query",
  Auth = "Auth",
  Header = "Header",
}

const tabClass = css`
  .n-tabs-tab__label {
    padding: 0 15px;
    .n-icon {
      margin-left: 5px;
    }
  }
  .hidden {
    display: none;
  }
  .format {
    position: absolute;
    right: 0;
    bottom: 0;
    .n-icon {
      font-size: 16px;
      font-weight: 900;
      margin-right: 5px;
    }
  }
`;

function shouldHaveBody(method: string) {
  return [HTTPMethod.POST, HTTPMethod.PUT, HTTPMethod.PATCH].includes(
    method as HTTPMethod
  );
}

export default defineComponent({
  name: "APISettingParamsReqParams",
  props: {
    params: {
      type: Object as PropType<HTTPRequest>,
      required: true,
    },
    onUpdateBody: {
      type: Function as PropType<(value: string) => void>,
      required: true,
    },
  },
  emits: ["update"],
  setup(props) {
    const settingStore = useSettingStore();
    const message = useMessage();
    const codeEditor = ref<Element>();
    const activeTab = ref("");
    let editor: EditorView;
    const destroy = () => {
      if (editor) {
        editor.destroy();
      }
    };
    const handleEditorUpdate = (v: ViewUpdate) => {
      if (v.docChanged) {
        props.onUpdateBody(editor.state.doc.toString().trim());
      }
    };
    const extensions = [
      basicSetup,
      keymap.of([indentWithTab]),
      json(),
      EditorView.updateListener.of(handleEditorUpdate),
    ];
    if (settingStore.isDark) {
      extensions.push(oneDark);
    }
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
          const trans = editor.state.update({
            changes: {
              from: 0,
              to: data.length,
              insert: result,
            },
          });
          editor.update([trans]);
        }
      } catch (err) {
        showError(message, err);
      }
    };
    // method变化时要选定对应的tab
    watch(
      () => props.params.method,
      (method) => {
        if (shouldHaveBody(method)) {
          activeTab.value = TabItem.Body;
        } else {
          activeTab.value = TabItem.Query;
        }
      }
    );
    onMounted(() => {
      initEditor();
      if (shouldHaveBody(props.params.method)) {
        activeTab.value = TabItem.Body;
      }
    });
    onBeforeUnmount(destroy);
    return {
      handleFormat,
      activeTab,
      codeEditor,
    };
  },
  render() {
    const { method } = this.$props.params;
    const { activeTab } = this;
    const tabs = [TabItem.Query, TabItem.Header, TabItem.Auth];
    if (shouldHaveBody(method)) {
      tabs.unshift(TabItem.Body);
    }
    const list = tabs.map((item) => {
      return <NTab name={item}>{item}</NTab>;
    });

    let codeEditorClass = "";
    if (activeTab !== TabItem.Body) {
      codeEditorClass = "hidden";
    }

    return (
      <div class={tabClass}>
        <NTabs
          key={method}
          type="line"
          defaultValue={tabs[0]}
          onUpdateValue={(value) => {
            this.activeTab = value;
          }}
        >
          {list}
        </NTabs>
        <div ref="codeEditor" class={codeEditorClass}>
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
        </div>
      </div>
    );
  },
});
