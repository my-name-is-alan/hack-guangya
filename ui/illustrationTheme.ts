import type { ConfigProviderProps } from 'antdv-next'
import { theme } from 'antdv-next'
import { createStaticStyles } from 'antdv-style'
import { computed } from 'vue'

const transition = 'all 0.2s cubic-bezier(0.4, 0, 0.2, 1)'
const popoverShadow = '0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -2px rgba(0, 0, 0, 0.1)'

const useStyles = createStaticStyles(({ css }) => ({
  buttonRoot: css({
    fontWeight: 500,
    transition,
  }),
  inputRoot: css({
    borderColor: '#e5e5e5',
    boxShadow: 'none',
    transition,
  }),
  selectRoot: css({
    '& .ant-select-selector': {
      borderColor: '#e5e5e5',
      boxShadow: 'none',
      transition,
    },
  }),
  modalContainer: css({
    border: '1px solid #e5e5e5',
    borderRadius: '14px',
    boxShadow: popoverShadow,
  }),
  popupBox: css({
    border: '1px solid #e5e5e5',
    borderRadius: '10px',
    backgroundColor: '#ffffff',
    boxShadow: popoverShadow,
  }),
  tooltipRoot: css({
    padding: 12,
  }),
  notificationRoot: css({
    '&.ant-notification-notice, & .ant-notification-notice': {
      border: '1px solid #e5e5e5',
      borderRadius: '10px',
      boxShadow: popoverShadow,
    },
  }),
  notificationTitle: css({
    fontWeight: 600,
  }),
  notificationDescription: css({
    color: '#525252',
  }),
  progressRail: css({
    backgroundColor: '#f4f4f5',
    boxShadow: 'none',
  }),
  progressTrack: css({
    border: 'none',
  }),
  inputNumberActions: css({
    width: '12px',
  }),
}))

function useIllustrationTheme() {
  const { styles } = useStyles()

  return computed<ConfigProviderProps>(() => ({
    componentSize: 'small',
    theme: {
      algorithm: theme.defaultAlgorithm,
      cssVar: true,
      token: {
        colorPrimary: '#262626',
        colorSuccess: '#22c55e',
        colorWarning: '#f97316',
        colorError: '#ef4444',
        colorInfo: '#262626',
        colorTextBase: '#262626',
        colorPrimaryBg: '#f5f5f5',
        colorPrimaryBgHover: '#e5e5e5',
        colorPrimaryBorder: '#d4d4d4',
        colorPrimaryBorderHover: '#a3a3a3',
        colorPrimaryHover: '#404040',
        colorPrimaryActive: '#171717',
        colorPrimaryText: '#262626',
        colorPrimaryTextHover: '#404040',
        colorPrimaryTextActive: '#171717',
        colorSuccessBg: '#f0fdf4',
        colorSuccessBgHover: '#dcfce7',
        colorSuccessBorder: '#bbf7d0',
        colorSuccessBorderHover: '#86efac',
        colorSuccessHover: '#16a34a',
        colorSuccessActive: '#15803d',
        colorSuccessText: '#16a34a',
        colorSuccessTextHover: '#16a34a',
        colorSuccessTextActive: '#15803d',
        colorWarningBg: '#fff7ed',
        colorWarningBgHover: '#fed7aa',
        colorWarningBorder: '#fdba74',
        colorWarningBorderHover: '#fb923c',
        colorWarningHover: '#ea580c',
        colorWarningActive: '#c2410c',
        colorWarningText: '#ea580c',
        colorWarningTextHover: '#ea580c',
        colorWarningTextActive: '#c2410c',
        colorErrorBg: '#fef2f2',
        colorErrorBgHover: '#fecaca',
        colorErrorBorder: '#fca5a5',
        colorErrorBorderHover: '#f87171',
        colorErrorHover: '#dc2626',
        colorErrorActive: '#b91c1c',
        colorErrorText: '#dc2626',
        colorErrorTextHover: '#dc2626',
        colorErrorTextActive: '#b91c1c',
        colorInfoBg: '#f5f5f5',
        colorInfoBgHover: '#e5e5e5',
        colorInfoBorder: '#d4d4d4',
        colorInfoBorderHover: '#a3a3a3',
        colorInfoHover: '#404040',
        colorInfoActive: '#171717',
        colorInfoText: '#262626',
        colorInfoTextHover: '#404040',
        colorInfoTextActive: '#171717',
        colorText: '#262626',
        colorTextSecondary: '#525252',
        colorTextTertiary: '#737373',
        colorTextQuaternary: '#a3a3a3',
        colorTextDisabled: '#a3a3a3',
        colorBgBase: '#ffffff',
        colorBgContainer: '#ffffff',
        colorBgElevated: '#ffffff',
        colorBgLayout: '#fafafa',
        colorBgSpotlight: 'rgba(38, 38, 38, 0.85)',
        colorBgMask: 'rgba(38, 38, 38, 0.45)',
        colorBorder: '#e5e5e5',
        colorBorderSecondary: '#f5f5f5',
        lineWidth: 1,
        lineWidthBold: 1,
        borderRadius: 10,
        borderRadiusXS: 2,
        borderRadiusSM: 6,
        borderRadiusLG: 14,
        controlHeight: 28,
        controlHeightSM: 22,
        controlHeightLG: 34,
        fontSize: 13,
        fontWeightStrong: 600,
        padding: 16,
        paddingSM: 12,
        paddingLG: 24,
        margin: 16,
        marginSM: 12,
        marginLG: 24,
        boxShadow: '0 1px 3px 0 rgba(0, 0, 0, 0.1), 0 1px 2px -1px rgba(0, 0, 0, 0.1)',
        boxShadowSecondary: popoverShadow,
      },
      components: {
        Button: {
          primaryShadow: 'none',
          defaultShadow: 'none',
          dangerShadow: 'none',
          defaultBorderColor: '#e4e4e7',
          defaultColor: '#18181b',
          defaultBg: '#ffffff',
          defaultHoverBg: '#f4f4f5',
          defaultHoverBorderColor: '#d4d4d8',
          defaultHoverColor: '#18181b',
          defaultActiveBg: '#e4e4e7',
          defaultActiveBorderColor: '#d4d4d8',
          borderRadius: 6,
          fontWeight: 500,
        },
        Input: {
          activeShadow: 'none',
          hoverBorderColor: '#a1a1aa',
          activeBorderColor: '#18181b',
          borderRadius: 6,
        },
        Select: {
          optionSelectedBg: '#f4f4f5',
          optionActiveBg: '#fafafa',
          optionSelectedFontWeight: 500,
          borderRadius: 6,
        },
        Alert: {
          borderRadiusLG: 8,
        },
        Modal: {
          borderRadiusLG: 12,
          boxShadow: 'none',
        },
        Progress: {
          circleTextColor: '#262626',
          defaultColor: '#18181b',
          remainingColor: '#f4f4f5',
        },
        Steps: {
          iconSize: 32,
        },
        Switch: {
          trackHeight: 22,
          trackMinWidth: 44,
          innerMinMargin: 4,
          innerMaxMargin: 24,
        },
        Checkbox: {
          borderRadiusSM: 4,
        },
        Slider: {
          trackBg: '#f4f4f5',
          trackHoverBg: '#e4e4e7',
          handleSize: 18,
          handleSizeHover: 20,
          railSize: 6,
        },
        ColorPicker: {
          borderRadius: 6,
        },
        Notification: {
          colorSuccessBg: '#f0fdf4',
          colorErrorBg: '#fef2f2',
          colorInfoBg: '#f5f5f5',
          colorWarningBg: '#fff7ed',
        },
        Layout: {
          bodyBg: '#fafafa',
          footerBg: '#fafafa',
          headerBg: '#ffffff',
          headerColor: '#18181b',
          siderBg: '#ffffff',
          triggerBg: '#f4f4f5',
          triggerColor: '#18181b',
        },
        Menu: {
          activeBarBorderWidth: 0,
          itemBg: 'transparent',
          itemHeight: 30,
          itemMarginBlock: 2,
          itemMarginInline: 6,
          subMenuItemBg: 'transparent',
        },
        Card: {
          bodyPadding: 16,
          bodyPaddingSM: 12,
          headerFontSize: 13,
          headerFontSizeSM: 12,
          headerHeight: 42,
          headerHeightSM: 36,
        },
        Table: {
          cellPaddingBlock: 6,
          cellPaddingBlockMD: 4,
          cellPaddingBlockSM: 4,
          cellPaddingInline: 10,
          cellPaddingInlineMD: 8,
          cellPaddingInlineSM: 8,
        },
        Tooltip: {
          borderRadius: 6,
        },
        Radio: {},
      },
    },
    button: {
      classes: {
        root: styles.buttonRoot,
      },
    },
    input: {
      classes: {
        root: styles.inputRoot,
      },
    },
    select: {
      classes: {
        root: styles.selectRoot,
        popup: {
          root: styles.popupBox,
        },
      },
    },
    modal: {
      classes: {
        container: styles.modalContainer,
      },
    },
    colorPicker: {
      arrow: false,
      classes: {
        root: styles.popupBox,
      },
    },
    popover: {
      classes: {
        container: styles.popupBox,
      },
    },
    tooltip: {
      arrow: false,
      classes: {
        root: styles.tooltipRoot,
        container: styles.popupBox,
      },
    },
    notification: {
      classes: {
        root: styles.notificationRoot,
        title: styles.notificationTitle,
        description: styles.notificationDescription,
      },
    },
    dropdown: {
      classes: {
        root: styles.popupBox,
      },
    },
    inputNumber: {
      classes: {
        root: styles.inputRoot,
        actions: styles.inputNumberActions,
      },
    },
    progress: {
      classes: {
        rail: styles.progressRail,
        track: styles.progressTrack,
      },
      styles: {
        rail: {
          height: '6px',
        },
        track: {
          height: '6px',
        },
      },
    },
    wave: {},
    app: {},
    card: {},
    alert: {},
    checkbox: {},
    datePicker: {},
    switch: {},
    radio: {},
    segmented: {},
  }))
}

export default useIllustrationTheme
