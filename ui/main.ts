import { createApp, type Component } from 'vue'
import { createPinia } from 'pinia'
import {
  Alert,
  App as AntApp,
  Avatar,
  Badge,
  Breadcrumb,
  BreadcrumbItem,
  Button,
  Card,
  CheckableTag,
  Checkbox,
  Col,
  Collapse,
  CollapsePanel,
  ConfigProvider,
  Descriptions,
  DescriptionsItem,
  Divider,
  Drawer,
  Dropdown,
  Empty,
  Flex,
  Form,
  FormItem,
  Input,
  InputNumber,
  InputPassword,
  Layout,
  LayoutContent,
  LayoutFooter,
  LayoutHeader,
  LayoutSider,
  Menu,
  Modal,
  Popconfirm,
  Progress,
  QRCode,
  RadioButton,
  RadioGroup,
  Result,
  Row,
  Segmented,
  Select,
  Skeleton,
  Space,
  Spin,
  Steps,
  Switch,
  Table,
  TabPane,
  Tabs,
  Tag,
  TextArea,
  Timeline,
  Tooltip,
} from 'antdv-next'
import 'antdv-next/dist/reset.css'
import RootApp from './RootApp.vue'
import { router } from './router'
import './styles.css'

const app = createApp(RootApp)
const pinia = createPinia()

app.use(pinia)
app.use(router)

const components = [
  Alert, AntApp, Avatar, Badge, Breadcrumb, BreadcrumbItem, Button, Card, CheckableTag,
  Checkbox, Col, Collapse, CollapsePanel, ConfigProvider, Descriptions, DescriptionsItem,
  Divider, Drawer, Dropdown, Empty, Flex, Form, FormItem, Input, InputNumber, InputPassword,
  Layout, LayoutContent, LayoutFooter, LayoutHeader, LayoutSider, Menu, Modal, Popconfirm,
  Progress, QRCode, RadioButton, RadioGroup, Result, Row, Segmented, Select, Skeleton, Space,
  Spin, Steps, Switch, Table, TabPane, Tabs, Tag, TextArea, Timeline, Tooltip,
]

components.forEach(component => app.component(component.name!, component as Component))
app.mount('#app')
