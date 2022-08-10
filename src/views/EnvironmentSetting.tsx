import { NCard } from "naive-ui";
import { defineComponent } from "vue";
import { css } from "@linaria/core";

import ExKeyValue from "../components/ExKeyValue";
import { i18nEnvironment } from "../i18n";

const dialogClass = css`
  max-width: 600px;
`;

export default defineComponent({
  name: "EnvironmentSetting",
  render() {
    return (
      <NCard title={i18nEnvironment("title")} class={dialogClass}>
        <ExKeyValue
          params={[]}
          onUpdateParams={(params) => {
            console.dir(params);
          }}
        />
      </NCard>
    );
  },
});
