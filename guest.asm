bits 16
org 0x1000

mov ax, 0
mov ds, ax
mov ss, ax
mov sp, 0x2000

mov dx, 0xe9
mov si, msg

.next:
    lodsb
    test al, al
    jz .done
    out dx, al
    jmp .next

.done:
    hlt

msg:
    db 'real mode ok', 0
