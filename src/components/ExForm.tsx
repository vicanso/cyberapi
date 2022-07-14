import { defineComponent, PropType, ref } from "vue";
import {
  FormInst,
  FormRules,
  FormItemRule,
  NForm,
  NFormItem,
  NInput,
  NButton,
  useMessage,
} from "naive-ui";
import { get, isArray } from "lodash-es";

import { i18nCommon } from "../i18n";
import { showError } from "../helpers/util";

export interface ExUpdateData {
  [key: string]: unknown;
}
export interface ExOnSubmit {
  (data: ExUpdateData): Promise<void>;
}

export interface ExFormItem {
  inputType?: "textarea" | "text" | "password";
  key: string;
  label: string;
  placeholer: string;
  defaultValue?: string;
  rule?: FormItemRule;
}

export default defineComponent({
  name: "ExForm",
  props: {
    formItems: {
      type: Array as PropType<ExFormItem[]>,
      required: true,
    },
    onSubmit: {
      type: Function as PropType<ExOnSubmit>,
      required: true,
    },
  },
  setup(props) {
    const message = useMessage();
    const formRef = ref<FormInst | null>(null);
    const data: ExUpdateData = {};
    const rules: FormRules = {};
    props.formItems.forEach((item) => {
      if (item.defaultValue) {
        data[item.key] = item.defaultValue;
      }
      if (item.rule) {
        rules[item.key] = item.rule;
      }
    });
    const formValue = ref(data);
    const submitting = ref(false);
    const submit = async () => {
      submitting.value = true;
      try {
        await formRef.value?.validate();
        if (props.onSubmit) {
          await props.onSubmit(data);
        }
      } catch (errors) {
        let err = errors;
        if (isArray(errors)) {
          err = get(errors, "[0][0].message");
        }
        showError(message, err);
      } finally {
        submitting.value = false;
      }
    };
    return {
      submitting,
      formValue,
      formRef,
      rules,
      submit,
    };
  },
  render() {
    const { formValue, rules, submit, submitting } = this;
    const { formItems } = this.$props;
    const items = formItems.map((item) => {
      return (
        <NFormItem label={item.label} path={item.key} key={item.key}>
          <NInput
            clearable
            type={item.inputType || "text"}
            defaultValue={item.defaultValue}
            placeholder={item.placeholer}
            onInput={(value) => {
              formValue[item.key] = value;
            }}
          />
        </NFormItem>
      );
    });
    return (
      <NForm model={formValue} rules={rules} ref="formRef">
        {items}
        <div class="tar">
          <NButton loading={submitting} onClick={() => submit()}>
            {i18nCommon("confirm")}
          </NButton>
        </div>
      </NForm>
    );
  },
});
