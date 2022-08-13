import { defineComponent, PropType, ref } from "vue";
import { css } from "@linaria/core";

import {
  NButton,
  NInput,
  NInputGroup,
  NSelect,
  NIcon,
  NDropdown,
  NGradientText,
} from "naive-ui";

import { i18nCollection } from "../../i18n";
import { HTTPRequest, HTTPMethod } from "../../commands/http_request";
import { useEnvironmentStore, ENVRegexp } from "../../stores/environment";
import { storeToRefs } from "pinia";
import { CodeSlashOutline } from "@vicons/ionicons5";
import { EnvironmentStatus } from "../../commands/environment";

const environmentSelectWidth = 50;
const wrapperClass = css`
  padding: 7px 4px 5px 0;
  overflow: hidden;
  .environmentSelect {
    width: ${environmentSelectWidth}px;
    float: left;
    .n-icon {
      font-size: 16px;
      font-weight: 900;
    }
  }
  .url {
    margin-left: ${environmentSelectWidth}px;
    .method {
      width: 120px;
    }
    .submit {
      width: 80px;
    }
  }
  .n-input,
  .n-base-selection-label {
    background-color: transparent !important;
  }
`;

const envLabelClass = css`
  padding: 0 5px;
  span {
    margin-left: 10px;
  }
`;

interface CuttingURIResult {
  env: string;
  uri: string;
}

function cuttingURI(uri: string): CuttingURIResult {
  const result = {
    env: "",
    uri,
  };
  const arr = ENVRegexp.exec(uri);
  if (arr?.length === 2) {
    result.env = arr[1];
    result.uri = uri.substring(arr[0].length);
  }
  return result;
}

export default defineComponent({
  name: "APISettingParamsURI",
  props: {
    params: {
      type: Object as PropType<HTTPRequest>,
      required: true,
    },
    onUpdateURI: {
      type: Function as PropType<(value: Map<string, unknown>) => void>,
      required: true,
    },
    onSumbit: {
      type: Function as PropType<() => void>,
      required: true,
    },
  },
  setup(props) {
    const environmentStore = useEnvironmentStore();

    const { environments } = storeToRefs(environmentStore);
    const uriResult = cuttingURI(props.params.uri);

    const currentURI = ref(uriResult.uri);
    const env = ref(uriResult.env);
    const method = ref(props.params.method);

    const handleUpdate = () => {
      let uri = currentURI.value;
      if (env.value) {
        uri = `{{${env.value}}}${uri}`;
      }
      const changed =
        uri !== props.params.uri || method.value !== props.params.metod;

      if (changed && props.onUpdateURI) {
        props.onUpdateURI({
          method: method.value,
          uri,
        });
      }
    };

    return {
      handleUpdate,
      environments,
      method,
      env,
      currentURI,
    };
  },
  render() {
    const { environments, currentURI, env, method } = this;
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
    // 只过滤启用的
    const envOptions = environments
      .filter((item) => item.enabled === EnvironmentStatus.Enabled)
      .map((item) => {
        return {
          label: `${item.name} | ${item.value}`,
          key: item.name,
        };
      });
    let envPrefix = "";
    if (env) {
      envPrefix = env.substring(0, 2).toUpperCase();
    }

    return (
      <div class={wrapperClass}>
        <div class="environmentSelect">
          <NDropdown
            trigger="click"
            options={envOptions}
            renderLabel={(option) => {
              const label = (option.label as string) || "";
              const arr = label.split(" | ");
              return (
                <span class={envLabelClass}>
                  {arr[0]}
                  <span class="font12">{arr[1]}</span>
                </span>
              );
            }}
            value={env}
            onSelect={(value) => {
              this.env = value;
              this.handleUpdate();
            }}
          >
            <NButton quaternary>
              {!envPrefix && (
                <NIcon>
                  <CodeSlashOutline />
                </NIcon>
              )}
              {envPrefix && <NGradientText>{envPrefix}</NGradientText>}
            </NButton>
          </NDropdown>
        </div>
        <div class="url">
          <NInputGroup>
            <NSelect
              class="method"
              consistentMenuWidth={false}
              options={options}
              placeholder={""}
              defaultValue={method || HTTPMethod.GET}
              onUpdateValue={(value) => {
                this.method = value;
                this.handleUpdate();
              }}
            />

            <NInput
              defaultValue={currentURI}
              placeholder={"http://test.com/users/v1/me"}
              clearable
              onBlur={() => {
                this.handleUpdate();
              }}
              onUpdateValue={(value) => {
                this.currentURI = value;
              }}
            />
            <NButton
              type="primary"
              class="submit"
              onClick={() => {
                this.$props.onSumbit();
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
