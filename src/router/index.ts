import { createRouter, createWebHistory } from "vue-router";
import DefaultLayout from "../layouts/DefaultLayout.vue";
import HomePage from "../pages/HomePage.vue";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      component: DefaultLayout,
      children: [{ path: "", name: "home", component: HomePage }],
    },
  ],
});

export default router;
