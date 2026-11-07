import { defineConfig } from '@playwright/test';
export default defineConfig({testDir:'./tests/browser',workers:1,timeout:60000,use:{baseURL:process.env.PUBLIC_URL||'http://localhost:8082',viewport:{width:1440,height:1000},launchOptions:{executablePath:process.env.CHROMIUM_EXECUTABLE_PATH}},reporter:'list'});
