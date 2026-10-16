# 本地设计系统（0.4.0 / schema 4）

本项目借鉴 [Penpot Design Systems](https://help.penpot.app/user-guide/design-systems/) 的令牌、主题、组件、变体、本地库及 Flex/Grid 概念，以 Vue 3 和原生 HTML 独立实现；不复用 Penpot 应用源码，也不读写 `.penpot` 文件。通信资源、绑定、Vue Flow 与 Rust/Python 运行核心继续保留。平台及实际验证范围见 [PLAN.md](PLAN.md) 与 [VERIFICATION.md](VERIFICATION.md)。

## 使用路径

在界面窗左侧打开“设计系统”。先创建令牌和主题，再将选中控件的视觉属性绑定到令牌。令牌可影响颜色、字体、尺寸、圆角、阴影等视觉属性；业务绑定仍优先，令牌不能自动打开串口、启动流程或调用 Python。

选择一个根控件（其子控件自动包含），创建主组件；随后插入独立实例。在属性面板编辑实例属性会记录覆写，更新源定义时保留这些覆写和实例的绑定、事件及稳定 ID。可重置覆写、切换声明的变体组合或解除组件关联。删除被流程或绑定引用的子控件必须先处理引用，否则整个操作回退。工程撤销与重做包含设计系统操作。

本地库为 `.hmidlib.json`，以命名空间和内容版本导入，保存令牌集、主题、主组件与引用资源的字节快照。导入前显示版本及资源变化概况，确认后建立新快照；已有实例继续引用原来的定义。需要替换源时，通过主组件源编辑显式应用。库导入是工程变更；不连接远端、不隐式升级。工程保存和独立包导出使用已固定的定义及普通实例控件，离线运行无需安装设计库。导入外部库前应检查其内容；Python 与协议插件遵循现有可信工程执行边界。

## 模型和兼容

旧 schema 1/2/3 继续读取。使用新设计系统时工程提升至 schema 4，旧版本 Runner 不支持。核心字段如下：

```ts
designSystem: {
  version: 1,
  tokenSets: { [id]: { id, name, tokens: { [name]: { $type, $value, $description? } } } },
  themes: { [id]: { id, name, tokenSets: string[] } },
  activeThemeId?: string,
  components: { [id]: { id, name, rootWidgetId, widgets, variants? } },
  libraries: { [id]: { id, name, version, namespace } }
}
widget.styleTokens?: { [visualProperty]: tokenReference }
widget.componentInstance?: {
  componentId, instanceId, rootWidgetId,
  widgetIdMap: { [sourceWidgetId]: instanceWidgetId },
  variant?: { [axis]: value },
  overrides?: { [sourceWidgetId]: { [property]: value } }
}
```

实例在页面内物化为普通控件，定义和实例 ID 分离；新增子控件分配新 ID，其余映射保持稳定。父子坐标使用相对坐标，实例移动只改变根位置。变体包含 `axes`、合法 `combinations` 及可选 `values`；缺失轴值或不存在的组合明确拒绝，不自动猜测。

令牌类型支持 `color/dimension/number/boolean/typography/shadow/fontFamily/opacity/radius`。采用 DTCG JSON 的类型和值形式及别名支持子集，不承诺完整 DTCG 标准：

- 引用使用 `setId.tokenName` 固定到指定集合，跨主题仍可读取；未限定名称按当前主题中令牌集的顺序覆盖，无主题时按集合 ID 排序。切换主题校验当前页面的令牌引用，不完整的主题切换事务会回退。导入库内主组件的未限定引用会固定到库源主题的集合，避免被宿主同名令牌意外覆盖。
- `$value: "{color.primary}"` 优先引用同集令牌，`{base.color.primary}` 可跨集引用。别名保持类型一致，循环或悬空引用拒绝。
- 尺寸支持数值、px/rem 字符串和 `{value,unit}`，rem 统一换算为 16px；结构颜色支持 sRGB。字体及阴影投影为 CSS，其他颜色空间、单位和任意数学表达式不在本版范围。
- 字体复合令牌先展开，显式单字段令牌再覆盖；运行绑定和运行 UI 值仍具有更高优先级。

## 布局

保留绝对布局、旧 `padding:number`、`columns:number`，扩展 `direction/wrap/justify/align/gapX/gapY/paddingSides/rows/columnTracks/rowTracks`。网格轨道为 `{kind:'px'|'fr',value}` 或 `{kind:'auto'}`；控件 `layoutItem.size.width/height.mode` 可设 `fixed/fit/fill`。分侧 padding 优先于统一 padding。设计器、预览和 Runner 共用 `src/lib/widgets.ts` 的布局计算。

## 业务逻辑保持独立

通信仍作为工程资源配置；按钮事件、绑定和流程以稳定控件 ID 连接。设计系统更新不能修改物理设备状态。schema 4 同时提供工程 Python 模块 `codeModules`、USB Printer 通用传输资源、启动/分派节点和可带路径的变量引用；用户可在自己的工程中组合这些能力，接口见 [IMPLEMENTATION.md](IMPLEMENTATION.md)。默认工程为空白，不预置业务示例。
