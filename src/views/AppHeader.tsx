import { NBreadcrumb, NBreadcrumbItem, NDivider } from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent } from "vue";
import { storeToRefs } from "pinia";

import { mainHeaderHeight } from "../constants/style";
import { i18nCommon } from "../i18n";
import { names } from "../router/routes";
import { goTo } from "../router";
import { useHeaderStore } from "../stores/header";

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
  name: "AppHeaderView",
  setup() {
    const headerStore = useHeaderStore();

    const { breadcrumbs } = storeToRefs(headerStore);
    return {
      breadcrumbs,
    };
  },
  render() {
    const { breadcrumbs, $route } = this;
    const arr = [
      {
        route: names.home,
        name: i18nCommon("dashboard"),
      },
    ];
    arr.push(...breadcrumbs);
    const items = arr.map((item) => {
      return (
        <NBreadcrumbItem
          key={item.route}
          onClick={() => {
            if (item.route === $route.name) {
              return;
            }
            goTo(item.route);
          }}
        >
          {item.name}
        </NBreadcrumbItem>
      );
    });
    return (
      <header class={headerClass}>
        <div class={logoClass}>
          {i18nCommon("app")}
          <NDivider vertical />
        </div>
        <NBreadcrumb class={breadcrumbClass}>{items}</NBreadcrumb>
      </header>
    );
  },
});
