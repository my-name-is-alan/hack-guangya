import { createApp } from 'vue';
import {
  Alert,
  App as AntApp,
  Avatar,
  Badge,
  Button,
  Card,
  Checkbox,
  Col,
  ConfigProvider,
  Divider,
  Drawer,
  Empty,
  Flex,
  Form,
  FormItem,
  Input,
  InputPassword,
  InputNumber,
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
  Result,
  Row,
  Select,
  Skeleton,
  Space,
  Steps,
  Switch,
  Table,
  Tag,
  TextArea,
  Timeline,
  Tooltip,
} from 'antdv-next';
import 'antdv-next/dist/reset.css';
import App from './App.vue';
import './styles.css';

const app = createApp(App);
[
  Alert, AntApp, Avatar, Badge, Button, Card, Checkbox, Col, ConfigProvider, Divider, Drawer,
  Empty, Flex, Form, FormItem, Input, InputPassword, Layout, LayoutContent,
  InputNumber, LayoutFooter, LayoutHeader, LayoutSider, Menu, Modal, Popconfirm, Progress,
  QRCode, Result, Row, Select, Skeleton, Space, Steps, Switch, Table, Tag,
  TextArea, Timeline, Tooltip,
].forEach((component) => app.use(component));
app.mount('#app');
