import { defineComponent, PropType } from "vue";
import { css } from "@linaria/core";

import { NButton, NInput, NInputGroup, NSelect } from "naive-ui";

import { i18nCollection } from "../../i18n";
import { HTTPRequest, HTTPMethod } from "../../commands/http_request";

const methodWidth = 100;
const wrapperClass = css`
  padding: 7px 4px 5px 0;
  overflow: hidden;
  .method {
    width: ${methodWidth}px;
    float: left;
  }
  .url {
    margin-left: ${methodWidth}px;
    .submit {
      width: 80px;
    }
  }
  .n-input,
  .n-base-selection-label {
    background-color: transparent !important;
  }
`;

export default defineComponent({
  name: "APISettingParamsURI",
  props: {
    params: {
      type: Object as PropType<HTTPRequest>,
      required: true,
    },
  },
  emits: ["update"],
  setup(props) {
    return {
      currentURI: props.params.uri,
    };
  },
  render() {
    const { method, uri } = this.$props.params;
    const options = [
      HTTPMethod.GET,
      HTTPMethod.POST,
      HTTPMethod.PUT,
      HTTPMethod.PATCH,
      HTTPMethod.DELETE,
      HTTPMethod.OPTIONS,
      HTTPMethod.HEAD,
    ].map((item) => {
      return {
        label: item,
        value: item,
      };
    });

    return (
      <div class={wrapperClass}>
        <div class="method">
          <NSelect
            consistentMenuWidth={false}
            options={options}
            bordered={false}
            placeholder={""}
            defaultValue={method || "GET"}
            onUpdateValue={(value) => {
              this.$emit("update", {
                method: value,
              });
            }}
          />
        </div>
        <div class="url">
          <NInputGroup>
            {/* TODO 添加env的选择 */}
            <NInput
              defaultValue={uri}
              placeholder={"http://test.com/users/v1/me"}
              clearable
              onBlur={() => {
                if (this.currentURI !== uri) {
                  this.$emit("update", {
                    uri: this.currentURI,
                  });
                }
              }}
              onUpdateValue={(value) => {
                this.currentURI = value;
              }}
            />
            <NButton
              type="primary"
              class="submit"
              onClick={() => {
                console.dir("TODO");
              }}
            >
              {i18nCollection("send")}
            </NButton>
          </NInputGroup>
        </div>
      </div>
    );
  },
});
