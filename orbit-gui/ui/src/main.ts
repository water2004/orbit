import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import Library from "./pages/Library.vue";
import Browse from "./pages/Browse.vue";
import Instance from "./pages/Instance.vue";
import InstanceContext from "./components/InstanceContext.vue";
import Accounts from "./pages/Accounts.vue";
import Settings from "./pages/Settings.vue";
import Create from "./pages/Create.vue";
import "./styles.css";
const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: Library },
    { path: "/browse", component: Browse },
    {
      path: "/instance/:id",
      component: InstanceContext,
      children: [
        { path: "create", component: Create },
        { path: ":tab", component: Instance },
      ],
    },
    { path: "/accounts", component: Accounts },
    { path: "/settings", component: Settings },
    { path: "/create", component: Create },
  ],
});
createApp(App).use(router).mount("#app");
