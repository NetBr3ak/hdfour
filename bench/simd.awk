/^_.*:([[:space:]]*([#;].*|\/\/.*)?)?$/ {
    printing = /swap_in_place/
    if (printing) found = 1
}
printing { print }
printing && (/^\.?Lfunc_end/ || /^[[:space:]]*\.cfi_endproc/) { printing = 0 }
END { if (!found) exit 1 }
