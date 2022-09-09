import { EditorState } from "@codemirror/state";
import { EditorView } from "codemirror";
import { DialogApiInjection } from "naive-ui/es/dialog/src/DialogProvider";
import {
  defineComponent,
  onBeforeUnmount,
  onMounted,
  ref,
  PropType,
} from "vue";
import { i18nCollection, i18nCommon } from "../i18n";
import { useSettingStore } from "../stores/setting";
import ExForm, { ExFormItem, ExUpdateData } from "./ExForm";
import { getDefaultExtensions, replaceContent } from "../helpers/editor";
import { css } from "@linaria/core";
import { NButton, useMessage } from "naive-ui";
import { SettingType, useAPISettingStore } from "../stores/api_setting";
import { useAPIFolderStore } from "../stores/api_folder";
import { showError } from "../helpers/util";

interface OnConfirm {
  (data: ExUpdateData): Promise<void>;
}
interface DialogOption {
  dialog: DialogApiInjection;
  title: string;
  enterTriggerSubmit?: boolean;
  formItems: ExFormItem[];
  onConfirm: OnConfirm;
}

interface ImportDialogOption {
  dialog: DialogApiInjection;
  collection: string;
  folder?: string;
  data: string;
}
export default function newDialog(option: DialogOption) {
  const { dialog, formItems } = option;
  const d = dialog.info({
    title: option.title,
    autoFocus: true,
    closable: false,
    content: () => (
      <ExForm
        enterTriggerSubmit={option.enterTriggerSubmit}
        formItems={formItems}
        onSubmit={async (data) => {
          await option.onConfirm(data);
          d.destroy();
        }}
      />
    ),
  });
}

const codeEditorClass = css`
  .codeEditor {
    height: 300px;
    overflow-y: auto;
  }
`;

const ImportEditor = defineComponent({
  name: "CodeEditor",
  props: {
    data: {
      type: String,
      default: () => "",
    },
    collection: {
      type: String,
      required: true,
    },
    folder: {
      type: String,
      default: () => "",
    },
    onConfirm: {
      type: Function as PropType<() => void>,
      required: true,
    },
  },
  setup(props) {
    const settingStore = useSettingStore();
    const apiSettingStore = useAPISettingStore();
    const apiFolderStore = useAPIFolderStore();
    const message = useMessage();
    let editor: EditorView;
    const destroy = () => {
      if (editor) {
        editor.destroy();
      }
    };

    const codeEditor = ref<Element>();
    const extensions = getDefaultExtensions({
      isDark: settingStore.isDark,
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
      replaceContent(editor, props.data);
    };
    const processing = ref(false);
    const handleImport = async () => {
      if (processing.value) {
        return;
      }
      processing.value = true;
      const data = editor.state.doc.toString();
      try {
        const result = JSON.parse(data);
        const arr = Array.isArray(result) ? result : [result];
        for (let i = 0; i < arr.length; i++) {
          const item = arr[i];
          item.collection = props.collection;
          const category = item.category as string;
          if (category === SettingType.HTTP) {
            await apiSettingStore.add(item);
          } else {
            await apiFolderStore.add(item);
          }
          if (props.folder) {
            await apiFolderStore.addChild({
              id: props.folder,
              children: [item.id as string],
            });
          }
        }
        if (props.onConfirm) {
          props.onConfirm();
        }
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    };
    onMounted(() => {
      initEditor();
    });
    onBeforeUnmount(() => {
      destroy();
    });
    return {
      handleImport,
      processing,
      codeEditor,
    };
  },
  render() {
    return (
      <div class={codeEditorClass}>
        <div ref="codeEditor" class="codeEditor"></div>
        <div class="tar">
          <NButton
            loading={this.processing}
            onClick={() => {
              this.handleImport();
            }}
          >
            {i18nCommon("confirm")}
          </NButton>
        </div>
      </div>
    );
  },
});

export function newImportDialog(option: ImportDialogOption) {
  const { dialog, data, collection, folder } = option;
  const d = dialog.info({
    title: i18nCollection("importSettings"),
    closable: false,
    style: {
      width: "700px",
    },
    content: () => (
      <ImportEditor
        data={data}
        collection={collection}
        folder={folder}
        onConfirm={() => {
          d.destroy();
        }}
      />
    ),
  });
}
