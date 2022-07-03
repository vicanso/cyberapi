import { defineComponent } from "vue";
import { css } from "@linaria/core";

import "./main.css";
import APISettings from "./components/APISettings";

const appSettingClass = css`
  width: 300px;
`;

export default defineComponent({
  name: "App",
  render() {
    return (
      <div class={appSettingClass}>
        <APISettings />{" "}
      </div>
    );
  },
});
