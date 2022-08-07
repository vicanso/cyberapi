import { NTab, NTabs } from "naive-ui";
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
import { EditorView, keymap } from "@codemirror/view";

import { EditorState } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";
import { indentWithTab } from "@codemirror/commands";

import { HTTPMethod, HTTPRequest } from "../../commands/http_request";

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
  },
  setup(props) {
    const codeEditor = ref<Element>();
    const activeTab = ref("");
    let editor: EditorView;
    const destroy = () => {
      if (editor) {
        console.dir(editor.state.doc.toString());
        editor.destroy();
      }
    };
    const initEditor = () => {
      const state = EditorState.create({
        doc: props.params.body,
        extensions: [basicSetup, keymap.of([indentWithTab]), json(), oneDark],
      });
      editor = new EditorView({
        state,
        parent: codeEditor.value,
      });
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
        <div ref="codeEditor" class={codeEditorClass}></div>
      </div>
    );
  },
});
