	.globl	__RNvNtCs68PyTno6oZk_6hdfour2nt13swap_in_place
	.p2align	2
__RNvNtCs68PyTno6oZk_6hdfour2nt13swap_in_place:
	.cfi_startproc
	cmp	x2, #2
	b.lo	LBB147_2
	udiv	x8, x1, x2
	mul	x8, x8, x2
	cmp	x2, x8
	b.ls	LBB147_3
LBB147_2:
	ret
LBB147_3:
	lsr	x9, x2, #1
	and	x10, x9, #0x7fffffffffffffe0
	and	x11, x2, #0x30
	and	x12, x9, #0x7ffffffffffffff8
	add	x13, x2, x0
	sub	x13, x13, #16
	add	x14, x0, #16
	sub	x15, x2, #8
	sub	x16, x2, #1
	b	LBB147_5
LBB147_4:
	add	x0, x0, x2
	add	x13, x13, x2
	add	x14, x14, x2
	sub	x8, x8, x2
	cmp	x2, x8
	b.hi	LBB147_2
LBB147_5:
	cmp	x2, #16
	b.hs	LBB147_7
	mov	x17, #0
	b	LBB147_16
LBB147_7:
	cmp	x2, #64
	b.hs	LBB147_9
	mov	x1, #0
	b	LBB147_13
LBB147_9:
	mov	x17, x14
	mov	x1, x13
	and	x3, x9, #0x7fffffffffffffe0
LBB147_10:
	ldp	q0, q1, [x17, #-16]
	ldp	q3, q2, [x1, #-16]
	rev64.16b	v2, v2
	ext.16b	v2, v2, v2, #8
	rev64.16b	v3, v3
	ext.16b	v3, v3, v3, #8
	stp	q2, q3, [x17, #-16]
	rev64.16b	v0, v0
	ext.16b	v0, v0, v0, #8
	rev64.16b	v1, v1
	ext.16b	v1, v1, v1, #8
	stp	q1, q0, [x1, #-16]
	sub	x1, x1, #32
	add	x17, x17, #32
	sub	x3, x3, #32
	cbnz	x3, LBB147_10
	cmp	x9, x10
	b.eq	LBB147_4
	and	x1, x9, #0x7fffffffffffffe0
	and	x17, x9, #0x7fffffffffffffe0
	cbz	x11, LBB147_16
LBB147_13:
	sub	x17, x15, x1
LBB147_14:
	ldr	d0, [x0, x1]
	ldr	d1, [x0, x17]
	rev64.8b	v1, v1
	str	d1, [x0, x1]
	rev64.8b	v0, v0
	str	d0, [x0, x17]
	add	x1, x1, #8
	sub	x17, x17, #8
	cmp	x12, x1
	b.ne	LBB147_14
	and	x17, x9, #0x7ffffffffffffff8
	cmp	x9, x12
	b.eq	LBB147_4
LBB147_16:
	sub	x1, x16, x17
LBB147_17:
	ldrb	w3, [x0, x17]
	ldrb	w4, [x0, x1]
	strb	w4, [x0, x17]
	strb	w3, [x0, x1]
	add	x17, x17, #1
	sub	x1, x1, #1
	cmp	x9, x17
	b.ne	LBB147_17
	b	LBB147_4
	.cfi_endproc

	.globl	__RNvNtCs68PyTno6oZk_6hdfour2nt15from_vdata_type
	.p2align	2
__RNvNtCs68PyTno6oZk_6hdfour2nt15from_vdata_type:
	.cfi_startproc
	stp	x20, x19, [sp, #-32]!
	.cfi_def_cfa_offset 32
	stp	x29, x30, [sp, #16]
