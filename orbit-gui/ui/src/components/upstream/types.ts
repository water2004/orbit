export type ButtonType =
  "base" | "colored" | "colored-text" | "outlined" | "quiet";

export type ButtonSize = "xs" | "sm" | "md" | "lg" | "xl";

export type ButtonInteraction = "surface" | "filled" | "none";

export type ButtonColor =
  "brand" | "red" | "orange" | "green" | "blue" | "purple" | "medal_promotion";

export type ButtonNativeType = "button" | "submit" | "reset";

export interface ButtonProps {
  type?: ButtonType;
  color?: ButtonColor;
  size?: ButtonSize;
  interaction?: ButtonInteraction;
  nativeType?: ButtonNativeType;
  disabled?: boolean;
  loading?: boolean;
}
