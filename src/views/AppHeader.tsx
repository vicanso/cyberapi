import { NBreadcrumb, NBreadcrumbItem, NDivider } from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent } from "vue";

import { mainHeaderHeight } from "../constants/style";
import { i18nCommon } from "../i18n";

const headerClass = css`
  height: ${mainHeaderHeight}px;
  line-height: ${mainHeaderHeight}px;
`;

const logoClass = css`
  margin-left: 15px;
  margin-right: 10px;
  float: left;
  font-weight: 600;
  div {
    margin-left: 20px !important;
  }
`;
const breadcrumbClass = css`
  padding-top: ${(mainHeaderHeight - 25) / 2}px;
`;

export default defineComponent({
  name: "AppHeader",
  render() {
    return (
      <header class={headerClass}>
        <div class={logoClass}>
          {i18nCommon("app")}
          <NDivider vertical />
        </div>
        <NBreadcrumb class={breadcrumbClass}>
          <NBreadcrumbItem>{i18nCommon("dashboard")}</NBreadcrumbItem>
          <NBreadcrumbItem>collection</NBreadcrumbItem>
        </NBreadcrumb>
      </header>
    );
  },
});
