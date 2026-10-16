export interface MenuItem { label:string;action?:()=>void;disabled?:boolean;checked?:boolean;shortcut?:string;items?:MenuItem[];separator?:boolean }
