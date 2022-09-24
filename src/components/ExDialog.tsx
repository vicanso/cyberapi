import { EditorState } from "@codemirror/state";
import { EditorView } from "codemirror";
import { DialogApiInjection } from "naive-ui/es/dialog/src/DialogProvider";
import { defineComponent, onBeforeUnmount, ref, PropType } from "vue";
import { i18nCollection, i18nCommon } from "../i18n";
import { useSettingStore } from "../stores/setting";
import ExForm, { ExFormItem, ExUpdateData } from "./ExForm";
import { getDefaultExtensions, replaceContent } from "../helpers/editor";
import { css } from "@linaria/core";
import {
  NButton,
  NIcon,
  NTabPane,
  NTabs,
  NText,
  NUpload,
  NUploadDragger,
  UploadFileInfo,
  useMessage,
} from "naive-ui";
import { useAPISettingStore } from "../stores/api_setting";
import { useAPIFolderStore } from "../stores/api_folder";
import { getNormalDialogStyle, isJSON, showError } from "../helpers/util";
import { CloudUploadOutline } from "@vicons/ionicons5";
import { importAPI, ImportCategory } from "../commands/import_api";

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

const importWrapperHeight = 300;
const codeEditorClass = css`
  .codeEditor {
    height: ${importWrapperHeight}px;
    overflow-y: auto;
  }
`;

const ImportEditor = defineComponent({
  name: "ImportEditor",
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

    const currentTab = ref(ImportCategory.Text);
    const fileData = ref("");

    let editor: EditorView | null;
    const destroyEditor = () => {
      if (editor) {
        editor.destroy();
        editor = null;
      }
    };

    const codeEditor = ref<Element>();
    const extensions = getDefaultExtensions({
      isDark: settingStore.isDark,
    });
    const initEditor = () => {
      if (editor) {
        return;
      }
      const state = EditorState.create({
        extensions,
      });
      editor = new EditorView({
        state,
        parent: codeEditor.value,
      });
      editor.dispatch({});
      const data = isJSON(props.data) ? props.data : "";
      replaceContent(editor, data);
    };
    const processing = ref(false);
    const handleImport = async () => {
      if (processing.value) {
        return;
      }
      processing.value = true;
      try {
        if (currentTab.value === ImportCategory.Text) {
          if (editor) {
            fileData.value = editor.state.doc.toString();
          } else {
            fileData.value = "";
          }
        }
        const topIDList = await importAPI({
          category: currentTab.value,
          collection: props.collection,
          fileData: fileData.value,
        });
        // 如果指定了目录
        if (props.folder && topIDList.length) {
          await apiFolderStore.addChild({
            id: props.folder,
            children: topIDList,
          });
        }

        // 重新加载数据，触发页面刷新
        await apiFolderStore.fetch(props.collection);
        await apiSettingStore.fetch(props.collection);
        message.info(i18nCollection("importSuccess"));

        if (props.onConfirm) {
          props.onConfirm();
        }
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    };

    const handleReadFile = (blob: Blob) => {
      const r = new FileReader();
      r.onload = () => {
        fileData.value = r.result as string;
      };
      r.onerror = () => {
        fileData.value = "";
        showError(message, new Error("read file fail"));
      };
      r.readAsText(blob);
    };
    onBeforeUnmount(() => {
      destroyEditor();
    });
    return {
      currentTab,
      fileData,
      destroyEditor,
      initEditor,
      handleReadFile,
      handleImport,
      processing,
      codeEditor,
    };
  },
  render() {
    const { currentTab } = this;
    const uploadWrapper = (
      <NUpload
        style={{
          height: `${importWrapperHeight}px`,
        }}
        onChange={(data: {
          file: UploadFileInfo;
          fileList: UploadFileInfo[];
        }) => {
          this.handleReadFile(data.fileList[0].file as Blob);
        }}
      >
        <NUploadDragger>
          <div
            style={{
              marginBottom: "10px",
            }}
          >
            <NIcon size="48">
              <CloudUploadOutline />
            </NIcon>
          </div>
          <NText>{i18nCollection("dragUploadTips")}</NText>
        </NUploadDragger>
      </NUpload>
    );
    return (
      <div>
        <NTabs
          defaultValue={currentTab}
          onUpdateValue={(value) => {
            this.fileData = "";
            this.currentTab = value;
          }}
        >
          <NTabPane
            name={ImportCategory.Text}
            tab="JSON"
            onVnodeMounted={() => {
              this.initEditor();
            }}
            onVnodeUnmounted={() => {
              this.destroyEditor();
            }}
          >
            <div class={codeEditorClass}>
              <div ref="codeEditor" class="codeEditor"></div>
            </div>
          </NTabPane>
          <NTabPane name={ImportCategory.File} tab="File/Postman/Insonmia">
            {uploadWrapper}
          </NTabPane>
        </NTabs>
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
    style: getNormalDialogStyle(),
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
