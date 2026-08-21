import { createRouter, createWebHistory } from "vue-router";
import HomePage from "../pages/HomePage.vue";

const router = createRouter({
  history: createWebHistory(),
  routes: [],
});

export default router;
routes: [
    {
        path: "/",
        component: HomePage
    }
]