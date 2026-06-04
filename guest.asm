bits 16
org 0x1000

mov ax, 0
mov ds, ax

mov dx, 0x3f8
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
    db 'serial ok', 0
    