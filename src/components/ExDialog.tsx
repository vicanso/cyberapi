import { DialogApiInjection } from "naive-ui/es/dialog/src/DialogProvider";
import ExForm, { ExFormItem, ExUpdateData } from "./ExForm";

interface OnConfirm {
  (data: ExUpdateData): Promise<void>;
}
interface DialogOption {
  dialog: DialogApiInjection;
  title: string;
  formItems: ExFormItem[];
  onConfirm: OnConfirm;
}

export default function newDialog(option: DialogOption) {
  const { dialog, formItems } = option;
  const d = dialog.info({
    title: option.title,
    autoFocus: true,
    closable: false,
    content: () => (
      <ExForm
        formItems={formItems}
        onSubmit={async (data) => {
          await option.onConfirm(data);
          d.destroy();
        }}
      />
    ),
  });
}
