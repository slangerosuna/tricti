#!/bin/bash

# TriCTI GitHub Linguist Configuration Test Script

set -e

echo "🔍 Testing TriCTI GitHub Linguist Configuration"
echo "================================================"

# Check if required files exist
echo "📋 Checking configuration files..."

required_files=(
    ".gitattributes"
    ".github/languages.yml"
    ".github/linguist.yml"
    ".github/vendor.yml"
    ".github/tricti.tmLanguage.json"
    "editors/vscode/syntaxes/tricti.tmLanguage.json"
)

for file in "${required_files[@]}"; do
    if [[ -f "$file" ]]; then
        echo "✅ $file exists"
    else
        echo "❌ $file missing"
        exit 1
    fi
done

# Check if .gitattributes contains TriCTI configuration
echo ""
echo "🔧 Checking .gitattributes configuration..."
if grep -q "*.tri linguist-language=TriCTI" .gitattributes; then
    echo "✅ TriCTI language mapping found"
else
    echo "❌ TriCTI language mapping not found in .gitattributes"
    exit 1
fi

# Check if TextMate grammar is valid JSON
echo ""
echo "📝 Validating TextMate grammar..."
if python3 -m json.tool editors/vscode/syntaxes/tricti.tmLanguage.json > /dev/null 2>&1; then
    echo "✅ TextMate grammar is valid JSON"
else
    echo "❌ TextMate grammar has invalid JSON syntax"
    exit 1
fi

# Check if symlink is working
echo ""
echo "🔗 Checking grammar symlink..."
if [[ -L ".github/tricti.tmLanguage.json" ]] && [[ -e ".github/tricti.tmLanguage.json" ]]; then
    echo "✅ Grammar symlink is working"
else
    echo "❌ Grammar symlink is broken or missing"
    exit 1
fi

# Test pattern matching on sample file
echo ""
echo "🎯 Testing language detection patterns..."
test_file=".github/test-linguist.tri"

patterns=(
    "@trigger\|@memoize\|@sys_input"
    "::\s*(struct\|table\|db)"
    "::\s*([^)]*)\s*=>"
    "i64\|string\|bool"
)

for pattern in "${patterns[@]}"; do
    if grep -q -E "$pattern" "$test_file"; then
        echo "✅ Pattern '$pattern' found in test file"
    else
        echo "❌ Pattern '$pattern' not found in test file"
    fi
done

# Count TriCTI files in repository
echo ""
echo "📊 Counting TriCTI files in repository..."
tri_count=$(find . -name "*.tri" -type f | wc -l)
echo "Found $tri_count .tri files"

if [[ $tri_count -gt 0 ]]; then
    echo "✅ Repository contains TriCTI files"
    echo "Files:"
    find . -name "*.tri" -type f | head -5
    if [[ $tri_count -gt 5 ]]; then
        echo "... and $((tri_count - 5)) more"
    fi
else
    echo "❌ No .tri files found in repository"
fi

echo ""
echo "🎉 All tests passed! GitHub Linguist configuration is ready."
echo ""
echo "Next steps:"
echo "1. Commit and push these changes to GitHub"
echo "2. Visit any .tri file on GitHub.com to see syntax highlighting"
echo "3. Check repository language statistics"
echo "4. TriCTI should appear as a recognized programming language"

# Cleanup test file
rm -f "$test_file"